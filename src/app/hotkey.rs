//! Connect hotkey (Phase 12): combination rules, text form, settings file.

use std::fs;
use std::path::{Path, PathBuf};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// A key as labelled on the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// `b'A'..=b'Z'`
    Letter(u8),
    /// `0..=9`
    Digit(u8),
    /// `1..=12`
    F(u8),
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub mods: Mods,
    pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    UnsupportedKey,
    NeedsModifier,
    UsedForTyping,
    ReservedByApps,
}

impl Reject {
    pub fn message(self) -> &'static str {
        match self {
            Self::UnsupportedKey => "Use a letter, digit, F1-F12 or Space",
            Self::NeedsModifier => "Needs Ctrl, Alt or Win",
            Self::UsedForTyping => "Used for typing",
            Self::ReservedByApps => "Reserved by apps",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeySetting {
    /// Not set: use `default_combo()`.
    Default,
    /// Switched off by the user.
    Off,
    Combo(Combo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub hotkey: HotkeySetting,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: HotkeySetting::Default,
        }
    }
}

pub fn default_combo() -> Combo {
    Combo {
        mods: Mods {
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
        },
        key: Key::Letter(b'B'),
    }
}

fn parse_key(s: &str) -> Option<Key> {
    let upper = s.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    match bytes {
        [c @ b'A'..=b'Z'] => Some(Key::Letter(*c)),
        [d @ b'0'..=b'9'] => Some(Key::Digit(d - b'0')),
        _ if upper == "SPACE" => Some(Key::Space),
        [b'F', rest @ ..] if !rest.is_empty() => {
            let n: u8 = std::str::from_utf8(rest).ok()?.parse().ok()?;
            (1..=12).contains(&n).then_some(Key::F(n))
        }
        _ => None,
    }
}

fn key_name(key: Key) -> String {
    match key {
        Key::Letter(c) => (c as char).to_string(),
        Key::Digit(d) => d.to_string(),
        Key::F(n) => format!("F{n}"),
        Key::Space => "Space".to_string(),
    }
}

/// Parses `ctrl+alt+B` (any case, any order, spaces allowed).
pub fn parse(s: &str) -> Option<Combo> {
    let mut mods = Mods::default();
    let mut key = None;
    for part in s.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            "win" | "super" => mods.win = true,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(parse_key(part)?);
            }
        }
    }
    Some(Combo { mods, key: key? })
}

fn join(c: &Combo, names: [&str; 4]) -> String {
    let flags = [c.mods.ctrl, c.mods.alt, c.mods.shift, c.mods.win];
    let mut parts: Vec<String> = names
        .iter()
        .zip(flags)
        .filter(|(_, on)| *on)
        .map(|(n, _)| n.to_string())
        .collect();
    parts.push(key_name(c.key));
    parts.join("+")
}

/// Text form for `settings.txt`, for example `ctrl+alt+B`.
pub fn to_setting(c: &Combo) -> String {
    join(c, ["ctrl", "alt", "shift", "win"])
}

/// Text for the popup, for example `Ctrl+Alt+B`.
pub fn display(c: &Combo) -> String {
    join(c, ["Ctrl", "Alt", "Shift", "Win"])
}

/// `global-hotkey` maps `Code::KeyZ` to `VK_Z`, the key labelled Z in the
/// current layout, so the labelled letter maps straight to its `Code`.
pub fn to_global(c: &Combo) -> HotKey {
    let mut mods = Modifiers::empty();
    if c.mods.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if c.mods.alt {
        mods |= Modifiers::ALT;
    }
    if c.mods.shift {
        mods |= Modifiers::SHIFT;
    }
    if c.mods.win {
        mods |= Modifiers::META;
    }
    let code_name = match c.key {
        Key::Letter(ch) => format!("Key{}", ch as char),
        Key::Digit(d) => format!("Digit{d}"),
        Key::F(n) => format!("F{n}"),
        Key::Space => "Space".to_string(),
    };
    let code: Code = code_name.parse().expect("every Key has a Code");
    HotKey::new(Some(mods), code)
}

/// Checks a combination the user pressed. `types_text` says whether it
/// produces a character on this keyboard (AltGr is Ctrl+Alt on Windows).
pub fn check(mods: Mods, key: Option<Key>, types_text: bool) -> Result<Combo, Reject> {
    let key = key.ok_or(Reject::UnsupportedKey)?;
    if !(mods.ctrl || mods.alt || mods.win) {
        return Err(Reject::NeedsModifier);
    }
    if types_text {
        return Err(Reject::UsedForTyping);
    }
    if mods.alt && !mods.ctrl && !mods.win {
        return Err(Reject::ReservedByApps);
    }
    let ctrl_only = mods.ctrl && !mods.alt && !mods.win;
    if ctrl_only && matches!(key, Key::Letter(_) | Key::Digit(_)) {
        return Err(Reject::ReservedByApps);
    }
    Ok(Combo { mods, key })
}

pub fn parse_settings(s: &str) -> Settings {
    let mut settings = Settings::default();
    for line in s.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "hotkey" {
            continue;
        }
        let value = value.trim();
        settings.hotkey = if value.is_empty() {
            HotkeySetting::Off
        } else if let Some(c) = parse(value) {
            HotkeySetting::Combo(c)
        } else {
            tracing::warn!("settings.txt: unknown hotkey {value:?}, using the default");
            HotkeySetting::Default
        };
    }
    settings
}

/// Only known keys are written; unknown lines of an old file are dropped.
pub fn render_settings(s: &Settings) -> String {
    match s.hotkey {
        HotkeySetting::Default => String::new(),
        HotkeySetting::Off => "hotkey=
"
        .to_string(),
        HotkeySetting::Combo(c) => format!(
            "hotkey={}
",
            to_setting(&c)
        ),
    }
}

/// Ignores a press within one second of the last accepted one, so a
/// double press does not toggle twice.
#[derive(Debug, Default)]
pub struct PressGuard {
    last: Option<std::time::Instant>,
}

impl PressGuard {
    pub fn accept(&mut self, now: std::time::Instant) -> bool {
        let guard = std::time::Duration::from_secs(1);
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < guard)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

pub fn settings_path(local_app_data: &Path) -> PathBuf {
    local_app_data.join("earbuds-tray").join("settings.txt")
}

/// Missing or unreadable file gives the defaults.
pub fn read_settings_from(path: &Path) -> Settings {
    fs::read_to_string(path)
        .map(|s| parse_settings(&s))
        .unwrap_or_default()
}

pub fn write_settings_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, render_settings(settings))
}

pub fn read_settings() -> Settings {
    match std::env::var_os("LOCALAPPDATA") {
        Some(dir) => read_settings_from(&settings_path(Path::new(&dir))),
        None => Settings::default(),
    }
}

pub fn write_settings(settings: &Settings) -> std::io::Result<()> {
    let dir = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA not found")
    })?;
    write_settings_to(&settings_path(Path::new(&dir)), settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: false,
        win: false,
    };
    const CTRL_ALT: Mods = Mods {
        ctrl: true,
        alt: true,
        shift: false,
        win: false,
    };
    const ALT: Mods = Mods {
        ctrl: false,
        alt: true,
        shift: false,
        win: false,
    };
    const WIN: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        win: true,
    };

    fn combo(mods: Mods, key: Key) -> Combo {
        Combo { mods, key }
    }

    #[test]
    fn default_is_ctrl_alt_b() {
        assert_eq!(default_combo(), combo(CTRL_ALT, Key::Letter(b'B')));
        assert_eq!(display(&default_combo()), "Ctrl+Alt+B");
        assert_eq!(to_setting(&default_combo()), "ctrl+alt+B");
    }

    #[test]
    fn parse_forms() {
        assert_eq!(
            parse("ctrl+alt+B"),
            Some(combo(CTRL_ALT, Key::Letter(b'B')))
        );
        assert_eq!(
            parse(" Ctrl + Alt + b "),
            Some(combo(CTRL_ALT, Key::Letter(b'B')))
        );
        assert_eq!(parse("control+space"), Some(combo(CTRL, Key::Space)));
        assert_eq!(parse("win+F12"), Some(combo(WIN, Key::F(12))));
        assert_eq!(parse("ctrl+alt+7"), Some(combo(CTRL_ALT, Key::Digit(7))));
        assert_eq!(
            parse("shift+win+ctrl+alt+Z"),
            Some(combo(
                Mods {
                    ctrl: true,
                    alt: true,
                    shift: true,
                    win: true
                },
                Key::Letter(b'Z')
            ))
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        for s in [
            "",
            "ctrl+alt",
            "ctrl+alt+B+C",
            "ctrl+alt+F13",
            "ctrl+alt+F0",
            "ctrl+alt+Ä",
            "hyper+B",
            "garbage",
        ] {
            assert_eq!(parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn round_trip_every_key() {
        let mut keys: Vec<Key> = (b'A'..=b'Z').map(Key::Letter).collect();
        keys.extend((0..=9).map(Key::Digit));
        keys.extend((1..=12).map(Key::F));
        keys.push(Key::Space);
        let mods = Mods {
            ctrl: true,
            alt: false,
            shift: true,
            win: true,
        };
        for key in keys {
            let c = combo(mods, key);
            assert_eq!(parse(&to_setting(&c)), Some(c), "{c:?}");
            assert_eq!(parse(&display(&c)), Some(c), "{c:?}");
        }
    }

    #[test]
    fn display_names() {
        assert_eq!(display(&combo(CTRL, Key::Space)), "Ctrl+Space");
        assert_eq!(display(&combo(WIN, Key::F(5))), "Win+F5");
        assert_eq!(
            display(&combo(
                Mods {
                    ctrl: true,
                    alt: true,
                    shift: true,
                    win: true
                },
                Key::Digit(3)
            )),
            "Ctrl+Alt+Shift+Win+3"
        );
    }

    #[test]
    fn global_hotkey_uses_labelled_letter() {
        let h = to_global(&combo(CTRL_ALT, Key::Letter(b'Z')));
        assert_eq!(
            h,
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyZ)
        );
        let h = to_global(&combo(WIN, Key::Digit(0)));
        assert_eq!(h, HotKey::new(Some(Modifiers::META), Code::Digit0));
        let h = to_global(&combo(CTRL, Key::Space));
        assert_eq!(h, HotKey::new(Some(Modifiers::CONTROL), Code::Space));
        let h = to_global(&combo(CTRL, Key::F(11)));
        assert_eq!(h, HotKey::new(Some(Modifiers::CONTROL), Code::F11));
    }

    #[test]
    fn check_rules() {
        let b = Some(Key::Letter(b'B'));
        assert_eq!(
            check(CTRL_ALT, b, false),
            Ok(combo(CTRL_ALT, Key::Letter(b'B')))
        );
        assert_eq!(
            check(CTRL, Some(Key::Space), false),
            Ok(combo(CTRL, Key::Space))
        );
        assert_eq!(
            check(CTRL, Some(Key::F(4)), false),
            Ok(combo(CTRL, Key::F(4)))
        );
        assert_eq!(check(WIN, b, false), Ok(combo(WIN, Key::Letter(b'B'))));

        assert_eq!(check(CTRL_ALT, None, false), Err(Reject::UnsupportedKey));
        assert_eq!(check(Mods::default(), b, false), Err(Reject::NeedsModifier));
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(check(shift, b, false), Err(Reject::NeedsModifier));
        // AltGr+Q gives @ on German keyboards.
        assert_eq!(
            check(CTRL_ALT, Some(Key::Letter(b'Q')), true),
            Err(Reject::UsedForTyping)
        );
        assert_eq!(
            check(ALT, Some(Key::F(4)), false),
            Err(Reject::ReservedByApps)
        );
        assert_eq!(
            check(ALT, Some(Key::Space), false),
            Err(Reject::ReservedByApps)
        );
        let alt_shift = Mods { shift: true, ..ALT };
        assert_eq!(check(alt_shift, b, false), Err(Reject::ReservedByApps));
        assert_eq!(
            check(CTRL, Some(Key::Letter(b'C')), false),
            Err(Reject::ReservedByApps)
        );
        assert_eq!(
            check(CTRL, Some(Key::Digit(1)), false),
            Err(Reject::ReservedByApps)
        );
        let ctrl_shift = Mods {
            shift: true,
            ..CTRL
        };
        assert_eq!(
            check(ctrl_shift, Some(Key::Letter(b'T')), false),
            Err(Reject::ReservedByApps)
        );
    }

    #[test]
    fn reject_messages_are_short() {
        for r in [
            Reject::UnsupportedKey,
            Reject::NeedsModifier,
            Reject::UsedForTyping,
            Reject::ReservedByApps,
        ] {
            let m = r.message();
            assert!(!m.is_empty() && m.len() <= 40, "{m}");
        }
    }

    #[test]
    fn press_guard_ignores_double_press() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mut g = PressGuard::default();
        assert!(g.accept(t0));
        assert!(!g.accept(t0 + Duration::from_millis(300)));
        assert!(!g.accept(t0 + Duration::from_millis(999)));
        assert!(g.accept(t0 + Duration::from_millis(1000)));
        // Ignored presses do not extend the window.
        assert!(!g.accept(t0 + Duration::from_millis(1500)));
        assert!(g.accept(t0 + Duration::from_millis(2000)));
    }

    #[test]
    fn settings_parse() {
        assert_eq!(parse_settings("").hotkey, HotkeySetting::Default);
        assert_eq!(
            parse_settings("hotkey=ctrl+Space\n").hotkey,
            HotkeySetting::Combo(combo(CTRL, Key::Space))
        );
        assert_eq!(parse_settings("hotkey=\n").hotkey, HotkeySetting::Off);
        assert_eq!(parse_settings("hotkey=   ").hotkey, HotkeySetting::Off);
        assert_eq!(
            parse_settings("hotkey=garbage").hotkey,
            HotkeySetting::Default
        );
        assert_eq!(
            parse_settings("# note\r\ncolor=blue\r\nbroken line\r\nhotkey = win+F1\r\n").hotkey,
            HotkeySetting::Combo(combo(WIN, Key::F(1)))
        );
    }

    #[test]
    fn settings_render_round_trip() {
        for hotkey in [
            HotkeySetting::Default,
            HotkeySetting::Off,
            HotkeySetting::Combo(combo(CTRL_ALT, Key::Letter(b'B'))),
        ] {
            let s = Settings { hotkey };
            assert_eq!(parse_settings(&render_settings(&s)), s, "{hotkey:?}");
        }
        assert_eq!(
            render_settings(&Settings {
                hotkey: HotkeySetting::Off
            }),
            "hotkey=\n"
        );
    }

    #[test]
    fn settings_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("earbuds-tray-test-{}", std::process::id()));
        let path = settings_path(&dir);
        assert_eq!(
            read_settings_from(&path),
            Settings::default(),
            "missing file"
        );
        let s = Settings {
            hotkey: HotkeySetting::Combo(combo(CTRL, Key::Space)),
        };
        write_settings_to(&path, &s).unwrap();
        assert_eq!(read_settings_from(&path), s);
        let _ = fs::remove_dir_all(&dir);
    }
}
