use crate::app::status::{DeviceStatus, Level, Source, is_beta, level_for};
use crate::app::tiers::Vendor;

pub const HINT_UNSUPPORTED: &str =
    "Your earbuds are not yet supported! Please raise an issue on GitHub";
pub const NOTE_CASE_LAST_KNOWN: &str = "Case: last known value";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub pct: Option<u8>,
    pub level: Level,
    /// The value is not live (last known case value).
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub enabled: bool,
}

/// The hotkey row at the bottom of the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutRow {
    /// Shows the current shortcut (or `Off`) with a Change link.
    Idle(String),
    /// Waiting for a key press; holds why the last try was rejected.
    Recording(Option<String>),
}

pub const RECORDING_TEXT: &str = "Press a shortcut (Esc cancels, Del turns it off)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupModel {
    pub title: String,
    pub source: Option<&'static str>,
    pub status_line: Option<&'static str>,
    pub rows: Vec<Row>,
    pub note: Option<&'static str>,
    pub hint: Option<&'static str>,
    pub button: Button,
    /// Rounded remaining runtime, for example `~3 h left`. Set by the caller.
    pub runtime: Option<String>,
    /// Set by the caller when the hotkey feature is available.
    pub shortcut: Option<ShortcutRow>,
}

/// Builds what the popup shows. `toggle` is `LinkControl::toggle_label()`,
/// `has_device` says whether a real device is picked.
pub fn popup_model(
    device_name: Option<&str>,
    status: &DeviceStatus,
    toggle: (&'static str, bool),
    has_device: bool,
) -> PopupModel {
    let title = device_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Earbuds")
        .to_string();
    let (label, toggle_enabled) = toggle;
    let button = Button {
        label,
        enabled: toggle_enabled && has_device,
    };
    let mut model = PopupModel {
        title,
        source: None,
        status_line: None,
        rows: Vec::new(),
        note: None,
        hint: None,
        button,
        runtime: None,
        shortcut: None,
    };

    let (view, source) = match status {
        DeviceStatus::Connecting => {
            model.status_line = Some("Connecting...");
            return model;
        }
        DeviceStatus::Disconnected => {
            model.status_line = Some("Not connected");
            return model;
        }
        DeviceStatus::Connected { view, source } => (view, *source),
    };

    let beta = is_beta(source, &model.title);
    model.source = Some(match source {
        Source::Vendor(v) | Source::VendorHfp(v) => match (v, beta) {
            (Vendor::Edifier, false) => "Edifier",
            (Vendor::Edifier, true) => "Edifier (beta)",
            (Vendor::Galaxy, false) => "Galaxy",
            (Vendor::Galaxy, true) => "Galaxy (beta)",
        },
        Source::FastPair { .. } | Source::FastPairHfp => "Fast Pair",
        Source::Windows => "Windows",
    });
    if source == Source::Windows {
        model.hint = Some(HINT_UNSUPPORTED);
    }

    let Some(view) = view else {
        model.status_line = Some("Connected, waiting for battery data");
        return model;
    };

    match source {
        Source::Windows | Source::FastPairHfp | Source::VendorHfp(_) => {
            model.rows.push(make_row("Battery", view.left, false));
        }
        Source::Vendor(_) | Source::FastPair { .. } => {
            model.rows.push(make_row("Left", view.left, false));
            model.rows.push(make_row("Right", view.right, false));
            if view.case.is_some() {
                let stale = matches!(
                    source,
                    Source::FastPair {
                        case_last_known: true,
                        ..
                    }
                );
                model.rows.push(make_row("Case", view.case, stale));
                if stale {
                    model.note = Some(NOTE_CASE_LAST_KNOWN);
                }
            }
        }
    }
    model
}

fn make_row(label: &'static str, pct: Option<u8>, stale: bool) -> Row {
    Row {
        label,
        pct,
        level: pct.map_or(Level::Unknown, level_for),
        stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::status::{BatteryView, Source};

    fn connected(
        left: Option<u8>,
        right: Option<u8>,
        case: Option<u8>,
        source: Source,
    ) -> DeviceStatus {
        DeviceStatus::Connected {
            view: Some(BatteryView { left, right, case }),
            source,
        }
    }

    fn fp(case_last_known: bool) -> Source {
        Source::FastPair {
            model_id: None,
            case_last_known,
        }
    }

    fn row(label: &'static str, pct: Option<u8>, level: Level) -> Row {
        Row {
            label,
            pct,
            level,
            stale: false,
        }
    }

    #[test]
    fn title_uses_device_name_or_earbuds() {
        let m = popup_model(
            Some("NeoBuds Pro 3"),
            &DeviceStatus::Connecting,
            ("Connect", false),
            true,
        );
        assert_eq!(m.title, "NeoBuds Pro 3");
        let m = popup_model(None, &DeviceStatus::Connecting, ("Connect", false), false);
        assert_eq!(m.title, "Earbuds");
    }

    #[test]
    fn connecting_has_status_line_and_no_rows() {
        let m = popup_model(
            Some("X"),
            &DeviceStatus::Connecting,
            ("Connect", false),
            true,
        );
        assert_eq!(m.status_line, Some("Connecting..."));
        assert!(m.rows.is_empty());
        assert_eq!(m.source, None);
        assert_eq!(m.note, None);
        assert_eq!(m.hint, None);
    }

    #[test]
    fn disconnected_has_status_line() {
        let m = popup_model(
            Some("X"),
            &DeviceStatus::Disconnected,
            ("Connect", true),
            true,
        );
        assert_eq!(m.status_line, Some("Not connected"));
        assert!(m.rows.is_empty());
        assert_eq!(m.source, None);
    }

    #[test]
    fn connected_without_view_waits_for_data() {
        let s = DeviceStatus::Connected {
            view: None,
            source: Source::Vendor(Vendor::Edifier),
        };
        let m = popup_model(Some("NeoBuds Pro 3"), &s, ("Disconnect", true), true);
        assert_eq!(m.status_line, Some("Connected, waiting for battery data"));
        assert!(m.rows.is_empty());
        assert_eq!(m.source, Some("Edifier"));
        assert_eq!(m.hint, None);
    }

    #[test]
    fn windows_without_view_shows_hint() {
        let s = DeviceStatus::Connected {
            view: None,
            source: Source::Windows,
        };
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        assert_eq!(m.hint, Some(HINT_UNSUPPORTED));
        assert_eq!(m.source, Some("Windows"));
    }

    #[test]
    fn vendor_shows_left_right_case() {
        let s = connected(
            Some(80),
            Some(45),
            Some(15),
            Source::Vendor(Vendor::Edifier),
        );
        let m = popup_model(Some("NeoBuds Pro 3"), &s, ("Disconnect", true), true);
        assert_eq!(m.source, Some("Edifier"));
        assert_eq!(m.status_line, None);
        assert_eq!(
            m.rows,
            vec![
                row("Left", Some(80), Level::High),
                row("Right", Some(45), Level::Mid),
                row("Case", Some(15), Level::Low),
            ]
        );
        assert_eq!(m.note, None);
        assert_eq!(m.hint, None);
    }

    #[test]
    fn case_row_hidden_without_value() {
        let s = connected(Some(80), Some(45), None, Source::Vendor(Vendor::Edifier));
        let m = popup_model(Some("NeoBuds Pro 3"), &s, ("Disconnect", true), true);
        assert_eq!(m.rows.len(), 2);
        assert!(m.rows.iter().all(|r| r.label != "Case"));
    }

    #[test]
    fn unknown_bud_value_keeps_row() {
        let s = connected(Some(80), None, None, fp(false));
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        assert_eq!(
            m.rows,
            vec![
                row("Left", Some(80), Level::High),
                row("Right", None, Level::Unknown),
            ]
        );
        assert_eq!(m.source, Some("Fast Pair"));
    }

    #[test]
    fn fastpair_last_known_case_is_stale_with_note() {
        let s = connected(Some(80), Some(70), Some(55), fp(true));
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        let case = m.rows.iter().find(|r| r.label == "Case").unwrap();
        assert!(case.stale);
        assert_eq!(case.pct, Some(55));
        assert!(
            m.rows
                .iter()
                .filter(|r| r.label != "Case")
                .all(|r| !r.stale)
        );
        assert_eq!(m.note, Some(NOTE_CASE_LAST_KNOWN));
    }

    #[test]
    fn fastpair_live_case_has_no_note() {
        let s = connected(Some(80), Some(70), Some(55), fp(false));
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        assert!(m.rows.iter().all(|r| !r.stale));
        assert_eq!(m.note, None);
    }

    #[test]
    fn windows_shows_single_row_and_hint() {
        let s = connected(Some(60), None, None, Source::Windows);
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        assert_eq!(m.rows, vec![row("Battery", Some(60), Level::High)]);
        assert_eq!(m.hint, Some(HINT_UNSUPPORTED));
        assert_eq!(m.source, Some("Windows"));
    }

    #[test]
    fn fastpair_hfp_shows_single_row_without_hint() {
        let s = connected(Some(30), None, None, Source::FastPairHfp);
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        assert_eq!(m.rows, vec![row("Battery", Some(30), Level::Mid)]);
        assert_eq!(m.hint, None);
        assert_eq!(m.source, Some("Fast Pair"));
    }

    #[test]
    fn button_follows_toggle_and_device() {
        let s = DeviceStatus::Disconnected;
        let m = popup_model(Some("X"), &s, ("Connect", true), true);
        assert_eq!(
            m.button,
            Button {
                label: "Connect",
                enabled: true
            }
        );
        let m = popup_model(Some("X"), &s, ("Disconnect", true), true);
        assert_eq!(
            m.button,
            Button {
                label: "Disconnect",
                enabled: true
            }
        );
        let m = popup_model(Some("X"), &s, ("Connect", false), true);
        assert!(!m.button.enabled);
        let m = popup_model(None, &s, ("Connect", true), false);
        assert!(!m.button.enabled);
    }

    #[test]
    fn galaxy_shows_beta_label_with_left_right_case() {
        let s = connected(
            Some(100),
            Some(95),
            Some(88),
            Source::Vendor(Vendor::Galaxy),
        );
        let m = popup_model(Some("Galaxy Buds2 Pro"), &s, ("Disconnect", true), true);
        assert_eq!(m.source, Some("Galaxy (beta)"));
        assert_eq!(
            m.rows,
            vec![
                row("Left", Some(100), Level::High),
                row("Right", Some(95), Level::High),
                row("Case", Some(88), Level::High),
            ]
        );
        assert_eq!(m.hint, None);
    }

    #[test]
    fn galaxy_without_case_has_no_case_row() {
        let s = connected(Some(70), Some(95), None, Source::Vendor(Vendor::Galaxy));
        let m = popup_model(Some("Galaxy Buds"), &s, ("Disconnect", true), true);
        assert_eq!(m.source, Some("Galaxy (beta)"));
        assert!(m.rows.iter().all(|r| r.label != "Case"));
    }

    #[test]
    fn galaxy_hfp_shows_single_row_and_beta_label() {
        let s = connected(Some(30), None, None, Source::VendorHfp(Vendor::Galaxy));
        let m = popup_model(Some("Galaxy Buds2 Pro"), &s, ("Disconnect", true), true);
        assert_eq!(m.rows, vec![row("Battery", Some(30), Level::Mid)]);
        assert_eq!(m.source, Some("Galaxy (beta)"));
        assert_eq!(m.hint, None);
    }

    #[test]
    fn neobuds_pro_3_is_not_beta() {
        let s = connected(
            Some(80),
            Some(45),
            Some(15),
            Source::Vendor(Vendor::Edifier),
        );
        let m = popup_model(
            Some("EDIFIER NeoBuds Pro 3"),
            &s,
            ("Disconnect", true),
            true,
        );
        assert_eq!(m.source, Some("Edifier"));
    }

    #[test]
    fn other_edifier_model_is_beta() {
        let s = connected(
            Some(80),
            Some(45),
            Some(15),
            Source::Vendor(Vendor::Edifier),
        );
        let m = popup_model(Some("EDIFIER W820NB"), &s, ("Disconnect", true), true);
        assert_eq!(m.source, Some("Edifier (beta)"));
    }
}
