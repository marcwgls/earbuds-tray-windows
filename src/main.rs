#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use std::env;
use std::time::{Duration, Instant};
use tracing::{Level, info, warn};
use tracing_subscriber::FmtSubscriber;

use earbuds_tray::app::alert::{Bud, LowBatteryAlert, alert_text};
use earbuds_tray::app::autostart;
use earbuds_tray::app::bt_audio;
use earbuds_tray::app::dpi::tray_icon_size;
use earbuds_tray::app::hotkey::{self, Combo, HotkeySetting, Mods, PressGuard, Settings};
use earbuds_tray::app::icon::{Slot, render_slot_with_style, slot_value, slot_visible};
use earbuds_tray::app::link::{AudioState, Effect, LinkControl, audio_line};
use earbuds_tray::app::logging;
use earbuds_tray::app::notify;
use earbuds_tray::app::popup::model::{PopupModel, ShortcutRow, popup_model};
use earbuds_tray::app::popup::toggle::{PopupToggle, ToggleAction};
use earbuds_tray::app::popup::window::Popup;
use earbuds_tray::app::probe;
use earbuds_tray::app::runtime::RuntimeEstimator;
use earbuds_tray::app::status::{DeviceStatus, Source, tooltip};
use earbuds_tray::app::tiers::Vendor;
use earbuds_tray::app::tray_promote;
use earbuds_tray::app::worker::{self, Command};
use earbuds_tray::drivers::edifier::PROFILE;
use earbuds_tray::drivers::edifier::packet::Decoder;
use earbuds_tray::drivers::edifier::query::{
    Format, Query, is_answer, parse_battery_d0, parse_battery_f2,
};
use earbuds_tray::drivers::edifier::transport::EdifierTransport;
use earbuds_tray::drivers::profile::{CaseReporting, DeviceProfile};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tao::event::{ElementState, Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::keyboard::KeyCode;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_probe(device_filter: &str) -> Result<()> {
    info!(
        "Starting Edifier NeoBuds probe against device matching '{}'",
        device_filter
    );

    let transport = EdifierTransport::connect(device_filter)?;

    // Strict probe sequence per WearEars & Phase 1 plan:
    // 1. V2 + XOR (D0, then F2)
    // 2. V2 without XOR (D0, then F2)
    // 3. V1 (D0, then F2)
    let probe_attempts = [
        (Query::BatteryD0, Format::V2Xor, "V2 + XOR BatteryD0 (0xD0)"),
        (Query::BatteryF2, Format::V2Xor, "V2 + XOR BatteryF2 (0xF2)"),
        (
            Query::BatteryD0,
            Format::V2NoXor,
            "V2 (No XOR) BatteryD0 (0xD0)",
        ),
        (
            Query::BatteryF2,
            Format::V2NoXor,
            "V2 (No XOR) BatteryF2 (0xF2)",
        ),
        (Query::BatteryD0, Format::V1, "V1 BatteryD0 (0xD0)"),
        (Query::BatteryF2, Format::V1, "V1 BatteryF2 (0xF2)"),
    ];

    let mut first_hit = false;
    let mut matched_format: Option<Format> = None;

    for (query, fmt, name) in &probe_attempts {
        // Drain any stale buffered incoming bytes before new probe attempt
        while transport.rx().try_recv().is_ok() {}

        let xor = *fmt == Format::V2Xor;
        let mut decoder = Decoder::new(xor);

        let tx_bytes = query.encode(*fmt);
        info!(
            "[TX] Sending {} ({} bytes): {}",
            name,
            tx_bytes.len(),
            to_hex(&tx_bytes)
        );

        if let Err(e) = transport.send_query(*query, *fmt) {
            warn!("[TX ERROR] Failed to send {}: {e}", name);
            continue;
        }

        let start = Instant::now();
        let timeout = Duration::from_millis(1500);

        while start.elapsed() < timeout {
            let remaining = timeout.saturating_sub(start.elapsed());
            match transport.rx().recv_timeout(remaining) {
                Ok(rx_bytes) => {
                    info!(
                        "[RX] Received raw chunk ({} bytes): {}",
                        rx_bytes.len(),
                        to_hex(&rx_bytes)
                    );
                    let frames = decoder.push(&rx_bytes);
                    for frame in frames {
                        info!(
                            "[FRAME] Decoded {:?} CMD 0x{:02X} (Payload {} bytes): {}",
                            frame.version,
                            frame.cmd,
                            frame.payload.len(),
                            to_hex(&frame.payload)
                        );

                        if is_answer(*query, &frame) {
                            if frame.cmd == 0xD0
                                && let Some(bat) = parse_battery_d0(&frame.payload)
                                && let Some(left) = bat.left.filter(|&l| l <= 100)
                            {
                                info!(
                                    "🎉 [SUCCESS] Battery D0 decoded: Left/Single={}%, Charging={:?}",
                                    left, bat.charging
                                );
                                matched_format = Some(*fmt);
                                first_hit = true;
                            } else if frame.cmd == 0xF2
                                && let Some(bat) = parse_battery_f2(&frame.payload)
                            {
                                info!(
                                    "[F2] Decoded payload (after XOR): {} | Parsed: Left={:?}, Right={:?}, Case={:?}, BudInCase={}, Raw={}",
                                    to_hex(&frame.payload),
                                    bat.left,
                                    bat.right,
                                    bat.case,
                                    bat.bud_in_case,
                                    to_hex(&bat.raw)
                                );
                                let left_ok = bat.left.is_some_and(|l| l <= 100);
                                let right_ok = bat.right.is_some_and(|r| r <= 100);
                                if left_ok && right_ok {
                                    info!(
                                        "🎉 [SUCCESS] Battery F2 decoded: Left={}% Right={}% Case={:?} (BudInCase={})",
                                        bat.left.unwrap(),
                                        bat.right.unwrap(),
                                        bat.case,
                                        bat.bud_in_case
                                    );
                                    matched_format = Some(*fmt);
                                    first_hit = true;
                                }
                            } else if frame.cmd == 0xCC {
                                info!("[ANC STATE] Raw notification: {}", to_hex(&frame.payload));
                            }
                        } else {
                            if frame.cmd == 0xF2
                                && let Some(bat) = parse_battery_f2(&frame.payload)
                            {
                                info!(
                                    "[UNSOLICITED] F2 frame received while waiting for {:?}: Payload (after XOR): {} | Parsed: Left={:?}, Right={:?}, Case={:?}, BudInCase={}, Raw={}",
                                    query,
                                    to_hex(&frame.payload),
                                    bat.left,
                                    bat.right,
                                    bat.case,
                                    bat.bud_in_case,
                                    to_hex(&bat.raw)
                                );
                            } else {
                                info!(
                                    "[UNSOLICITED] Frame CMD 0x{:02X} received while waiting for {:?}: Payload ({} bytes): {}",
                                    frame.cmd,
                                    query,
                                    frame.payload.len(),
                                    to_hex(&frame.payload)
                                );
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("Socket read channel disconnected.");
                    break;
                }
            }
        }

        if first_hit {
            info!("Probe matched successfully with '{}'!", name);
            break;
        }
    }

    if !first_hit {
        warn!("None of the safe probe attempts returned a valid battery frame within timeout.");
        warn!(
            "If the device is connected, check if the Edifier smartphone app is currently holding the RFCOMM channel."
        );
    }

    info!("Entering passive 30s listening phase with periodic F2 polling every 10s...");
    let passive_xor = matched_format.is_none_or(|f| f == Format::V2Xor);
    let mut passive_decoder = Decoder::new(passive_xor);
    let listen_start = Instant::now();
    let listen_duration = Duration::from_secs(30);
    let poll_interval = Duration::from_secs(10);
    let mut last_poll = Instant::now();

    while listen_start.elapsed() < listen_duration {
        if last_poll.elapsed() >= poll_interval {
            info!("[PASSIVE POLL] 10s elapsed, querying F2 battery status...");
            let fmt = matched_format.unwrap_or(Format::V2Xor);
            if let Err(e) = transport.send_query(Query::BatteryF2, fmt) {
                warn!("[PASSIVE POLL ERROR] Failed to send F2 query: {e}");
            }
            last_poll = Instant::now();
        }

        let time_to_next_poll = poll_interval.saturating_sub(last_poll.elapsed());
        let time_to_listen_end = listen_duration.saturating_sub(listen_start.elapsed());
        let wait_time = time_to_next_poll.min(time_to_listen_end);

        if wait_time.is_zero() {
            continue;
        }

        match transport.rx().recv_timeout(wait_time) {
            Ok(rx_bytes) => {
                info!(
                    "[PASSIVE RX] ({} bytes): {}",
                    rx_bytes.len(),
                    to_hex(&rx_bytes)
                );
                let frames = passive_decoder.push(&rx_bytes);
                for frame in frames {
                    info!(
                        "[PASSIVE FRAME] Decoded {:?} CMD 0x{:02X} (Payload {} bytes): {}",
                        frame.version,
                        frame.cmd,
                        frame.payload.len(),
                        to_hex(&frame.payload)
                    );
                    if frame.cmd == 0xF2
                        && let Some(bat) = parse_battery_f2(&frame.payload)
                    {
                        info!(
                            "[PASSIVE F2] Decoded payload (after XOR): {} | Parsed: Left={:?}, Right={:?}, Case={:?}, BudInCase={}, Raw={}",
                            to_hex(&frame.payload),
                            bat.left,
                            bat.right,
                            bat.case,
                            bat.bud_in_case,
                            to_hex(&bat.raw)
                        );
                        let left_ok = bat.left.is_some_and(|l| l <= 100);
                        let right_ok = bat.right.is_some_and(|r| r <= 100);
                        if left_ok && right_ok {
                            info!(
                                "🎉 [PASSIVE SUCCESS] Battery F2: Left={}% Right={}% Case={:?} (BudInCase={})",
                                bat.left.unwrap(),
                                bat.right.unwrap(),
                                bat.case,
                                bat.bud_in_case
                            );
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Timeout slice reached, continue loop
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                info!("Socket disconnected.");
                break;
            }
        }
    }

    info!("Passive listening finished. Probe session complete.");
    Ok(())
}

#[derive(Debug)]
enum UserEvent {
    Status {
        device_name: Option<String>,
        status: DeviceStatus,
    },
    Audio(bt_audio::AudioEvent),
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    /// Connect/Disconnect, from the menu item or the popup button.
    Toggle,
    Hotkey(GlobalHotKeyEvent),
}

fn slot_name(slot: Slot, is_single: bool) -> &'static str {
    match slot {
        Slot::Left if is_single => "Battery",
        Slot::Left => "Left",
        Slot::Right => "Right",
        Slot::Case => "Case",
    }
}

/// Windows and vendor-pending values show one battery bar.
fn is_single_source(source: Option<Source>) -> bool {
    matches!(
        source,
        Some(Source::Windows | Source::FastPairHfp | Source::VendorHfp(_))
    )
}

/// Which case slots a source shows. Galaxy uses the same rule as Edifier:
/// the case slot is shown only while the case value is known (Galaxy Buds
/// without case value, and a case reported as 0, show no case slot).
fn profile_for(source: Option<Source>) -> DeviceProfile {
    match source {
        Some(Source::FastPair { .. }) => DeviceProfile {
            case: CaseReporting::WithBudInCase,
        },
        Some(Source::Windows | Source::FastPairHfp | Source::VendorHfp(_)) => DeviceProfile {
            case: CaseReporting::Never,
        },
        _ => PROFILE,
    }
}

/// The runtime goes on the second line: the tray cuts long tooltips at the
/// end, so it must come before the device and audio lines.
fn slot_tooltip(
    slot: Slot,
    val: Option<u8>,
    runtime: Option<&str>,
    full_tooltip: &str,
    is_single: bool,
) -> String {
    let slot_line = match val {
        Some(pct) => format!("{} {}%", slot_name(slot, is_single), pct),
        None => slot_name(slot, is_single).to_string(),
    };
    match runtime {
        Some(r) => format!("{slot_line}\n{r}\n{full_tooltip}"),
        None => format!("{slot_line}\n{full_tooltip}"),
    }
}

/// Sets the tooltip of every tray icon (status and audio events).
fn update_tooltips(
    tray_icons: &mut [(Slot, TrayIcon, bool)],
    status: &DeviceStatus,
    device_name: &str,
    audio: AudioState,
    runtime: Option<&str>,
) {
    let full_tip = format!("{}\n{}", tooltip(device_name, status), audio_line(audio));
    let is_single = is_single_source(status.source());
    for (slot, tray, _) in tray_icons.iter_mut() {
        let (val, _) = slot_value(*slot, status);
        let tip = slot_tooltip(*slot, val, runtime, &full_tip, is_single);
        let _ = tray.set_tooltip(Some(&tip));
    }
}

fn should_update_device(current: Option<&str>, new_name: Option<&str>) -> Option<String> {
    let trimmed = new_name?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if current == Some(trimmed) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_toggle_enabled(toggle_enabled: bool, has_device: bool) -> bool {
    toggle_enabled && has_device
}

fn full_popup_model(
    device_name: Option<&str>,
    status: &DeviceStatus,
    link: &LinkControl,
    runtime: &RuntimeEstimator,
    hotkeys: &Option<HotkeyCtl>,
) -> PopupModel {
    let mut m = popup_model(
        device_name,
        status,
        link.toggle_label(),
        device_name.is_some(),
    );
    m.runtime = runtime.line(Instant::now(), status);
    m.shortcut = hotkeys.as_ref().map(|h| h.row.clone());
    m
}

fn refresh_popup(
    popup: &mut Option<Popup>,
    device_name: Option<&str>,
    status: &DeviceStatus,
    link: &LinkControl,
    runtime: &RuntimeEstimator,
    hotkeys: &Option<HotkeyCtl>,
) {
    if let Some(p) = popup.as_mut() {
        p.set_model(full_popup_model(
            device_name,
            status,
            link,
            runtime,
            hotkeys,
        ));
    }
}

/// The connect hotkey: registration, recording in the popup, settings file.
struct HotkeyCtl {
    manager: GlobalHotKeyManager,
    /// The saved choice, `None` when switched off.
    wanted: Option<Combo>,
    /// Registered right now.
    active: Option<Combo>,
    /// What the popup shows.
    row: ShortcutRow,
    /// Modifier state of the popup window while recording.
    mods: Mods,
    guard: PressGuard,
}

impl HotkeyCtl {
    fn start() -> Option<Self> {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                warn!("Hotkey support unavailable: {e}");
                return None;
            }
        };
        let wanted = match hotkey::read_settings().hotkey {
            HotkeySetting::Default => Some(hotkey::default_combo()),
            HotkeySetting::Off => None,
            HotkeySetting::Combo(c) => Some(c),
        };
        let mut ctl = Self {
            manager,
            wanted,
            active: None,
            row: ShortcutRow::Idle(String::new()),
            mods: Mods::default(),
            guard: PressGuard::default(),
        };
        ctl.activate_wanted();
        Some(ctl)
    }

    fn register(&mut self, c: Combo) -> bool {
        match self.manager.register(hotkey::to_global(&c)) {
            Ok(()) => {
                info!("Hotkey {} registered.", hotkey::display(&c));
                self.active = Some(c);
                true
            }
            Err(e) => {
                warn!("Hotkey {} not registered: {e}", hotkey::display(&c));
                false
            }
        }
    }

    fn unregister_active(&mut self) {
        if let Some(c) = self.active.take()
            && let Err(e) = self.manager.unregister(hotkey::to_global(&c))
        {
            warn!("Hotkey {} not unregistered: {e}", hotkey::display(&c));
        }
    }

    /// Registers the saved choice again and shows it.
    fn activate_wanted(&mut self) {
        self.row = match self.wanted {
            None => ShortcutRow::Idle("Off".to_string()),
            Some(c) => {
                if self.register(c) {
                    ShortcutRow::Idle(hotkey::display(&c))
                } else {
                    ShortcutRow::Idle(format!("{} (in use)", hotkey::display(&c)))
                }
            }
        };
    }

    fn is_recording(&self) -> bool {
        matches!(self.row, ShortcutRow::Recording(_))
    }

    /// The current hotkey is unregistered while recording, so the same
    /// combination reaches the popup.
    fn start_recording(&mut self) {
        self.unregister_active();
        self.row = ShortcutRow::Recording(None);
    }

    fn cancel(&mut self) {
        if self.is_recording() {
            self.activate_wanted();
        }
    }

    fn turn_off(&mut self) {
        self.unregister_active();
        self.wanted = None;
        self.save(HotkeySetting::Off);
        self.row = ShortcutRow::Idle("Off".to_string());
    }

    fn try_keys(&mut self, key: Option<hotkey::Key>, types_text: bool) {
        match hotkey::check(self.mods, key, types_text) {
            Err(reject) => {
                info!("Hotkey recording: rejected ({reject:?}).");
                self.row = ShortcutRow::Recording(Some(reject.message().to_string()));
            }
            Ok(c) => {
                if self.register(c) {
                    self.wanted = Some(c);
                    self.save(HotkeySetting::Combo(c));
                    self.row = ShortcutRow::Idle(hotkey::display(&c));
                } else {
                    self.row = ShortcutRow::Recording(Some("In use by another app".to_string()));
                }
            }
        }
    }

    fn save(&self, hotkey: HotkeySetting) {
        if let Err(e) = hotkey::write_settings(&Settings { hotkey }) {
            warn!("Failed to save settings.txt: {e}");
        }
    }

    /// True when the event is a press of our hotkey outside the guard time.
    fn on_event(&mut self, event: &GlobalHotKeyEvent, now: Instant) -> bool {
        if event.state != HotKeyState::Pressed {
            return false;
        }
        let Some(c) = self.active else { return false };
        if event.id != hotkey::to_global(&c).id() {
            return false;
        }
        if !self.guard.accept(now) {
            info!("Hotkey: second press within 1 s ignored.");
            return false;
        }
        true
    }
}

fn is_modifier_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
    )
}

/// The key as labelled: letters and digits from the layout, F keys and
/// Space from the physical key.
fn recorded_key(event: &tao::event::KeyEvent) -> Option<hotkey::Key> {
    let f = match event.physical_key {
        KeyCode::F1 => Some(1),
        KeyCode::F2 => Some(2),
        KeyCode::F3 => Some(3),
        KeyCode::F4 => Some(4),
        KeyCode::F5 => Some(5),
        KeyCode::F6 => Some(6),
        KeyCode::F7 => Some(7),
        KeyCode::F8 => Some(8),
        KeyCode::F9 => Some(9),
        KeyCode::F10 => Some(10),
        KeyCode::F11 => Some(11),
        KeyCode::F12 => Some(12),
        KeyCode::Space => return Some(hotkey::Key::Space),
        _ => None,
    };
    if let Some(n) = f {
        return Some(hotkey::Key::F(n));
    }
    let tao::keyboard::Key::Character(s) = event.key_without_modifiers() else {
        return None;
    };
    let mut chars = s.chars();
    let c = chars.next()?.to_ascii_uppercase();
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_uppercase() {
        Some(hotkey::Key::Letter(c as u8))
    } else if c.is_ascii_digit() {
        Some(hotkey::Key::Digit(c as u8 - b'0'))
    } else {
        None
    }
}

fn run_tray(
    device_filter: Option<String>,
    tier_override: Option<earbuds_tray::app::tiers::TierOverride>,
) -> Result<()> {
    #[cfg(windows)]
    let _instance_mutex = {
        use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::core::w;

        struct SafeHandle(HANDLE);
        impl Drop for SafeHandle {
            fn drop(&mut self) {
                if !self.0.is_invalid() {
                    unsafe {
                        let _ = CloseHandle(self.0);
                    }
                }
            }
        }

        let handle = unsafe { CreateMutexW(None, true, w!("EarbudsTray")) };
        match handle {
            Ok(h) => {
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    info!("EarbudsTray is already running. Exiting.");
                    let _ = unsafe { CloseHandle(h) };
                    return Ok(());
                }
                SafeHandle(h)
            }
            Err(e) => {
                warn!("CreateMutexW failed: {e}");
                SafeHandle(HANDLE::default())
            }
        }
    };

    let app_data = std::env::var_os("LOCALAPPDATA");
    // Debug builds log to the console only (see main), so don't point at the file.
    let log_path_display = if cfg!(debug_assertions) {
        "console only in debug builds".to_string()
    } else {
        app_data
            .as_ref()
            .map(|p| {
                logging::log_path(std::path::Path::new(p))
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| "unknown".to_string())
    };
    info!(
        "EarbudsTray v{} starting (log: {})",
        env!("CARGO_PKG_VERSION"),
        log_path_display
    );
    match &device_filter {
        Some(f) => info!("Starting EarbudsTray with device override filter '{f}'"),
        None => info!("Starting EarbudsTray (auto device discovery)"),
    }

    if let Err(e) = notify::register_aumid() {
        warn!("Failed to register AUMID: {e}");
    }

    let current_exe = match std::env::current_exe() {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(e) => {
            warn!("Failed to get current_exe: {e}");
            String::new()
        }
    };

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Command>();

    let proxy_status = proxy.clone();
    let on_status = move |device_name: Option<String>, status: DeviceStatus| {
        let _ = proxy_status.send_event(UserEvent::Status {
            device_name,
            status,
        });
    };

    let (audio_tx, audio_rx) = std::sync::mpsc::channel::<bt_audio::AudioCmd>();
    let audio_filter = device_filter.clone().unwrap_or_default();
    let proxy_audio = proxy.clone();
    let on_audio_event = move |event: bt_audio::AudioEvent| {
        let _ = proxy_audio.send_event(UserEvent::Audio(event));
    };

    let proxy_menu = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy_menu.send_event(UserEvent::Menu(event));
    }));
    let proxy_tray = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy_tray.send_event(UserEvent::Tray(event));
    }));
    let proxy_hotkey = proxy.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |event| {
        let _ = proxy_hotkey.send_event(UserEvent::Hotkey(event));
    }));
    let proxy_toggle = proxy.clone();

    let mut worker_handle = Some(worker::spawn(
        device_filter.clone(),
        tier_override,
        on_status,
        cmd_rx,
    ));
    let mut audio_handle = Some(bt_audio::spawn(audio_filter, on_audio_event, audio_rx));

    let mut link = LinkControl::new();

    let menu = Menu::new();
    let initial_status = DeviceStatus::Connecting;
    let initial_name = device_filter.as_deref().unwrap_or("Earbuds");
    let initial_base_tip = tooltip(initial_name, &initial_status);
    let initial_full_tip = format!("{}\n{}", initial_base_tip, audio_line(link.audio()));
    let initial_menu_text = initial_base_tip.replace('\n', ": ");

    let status_item = MenuItem::new(initial_menu_text, false, None);
    let audio_item = MenuItem::new(audio_line(link.audio()), false, None);
    let sep = PredefinedMenuItem::separator();
    let (toggle_text, _) = link.toggle_label();
    // Until a device is picked the toggle stays disabled
    let toggle_item = MenuItem::new(toggle_text, false, None);
    let initial_autostart =
        autostart::is_enabled_for(autostart::read_run_value().as_deref(), &current_exe);
    let autostart_item = CheckMenuItem::new("Start with Windows", true, initial_autostart, None);
    let refresh_item = MenuItem::new("Refresh", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    menu.append(&status_item)?;
    menu.append(&audio_item)?;
    menu.append(&sep)?;
    menu.append(&toggle_item)?;
    menu.append(&autostart_item)?;
    menu.append(&refresh_item)?;
    menu.append(&quit_item)?;

    let initial_icon_size = tray_icon_size();
    let slot_configs = [
        (Slot::Case, "earbuds-case"),
        (Slot::Right, "earbuds-right"),
        (Slot::Left, "earbuds-left"),
    ];

    let initial_popup_name = device_filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut popup = match Popup::new(
        &event_loop,
        popup_model(
            initial_popup_name,
            &initial_status,
            link.toggle_label(),
            initial_popup_name.is_some(),
        ),
    ) {
        Ok(p) => Some(p),
        Err(e) => {
            warn!("Popup unavailable, left click opens the menu: {e:#}");
            None
        }
    };
    let mut popup_toggle = PopupToggle::default();

    let initial_is_single = is_single_source(initial_status.source());
    let mut tray_icons = Vec::new();
    for (slot, id) in slot_configs {
        let (val, state) = slot_value(slot, &initial_status);
        let rgba = render_slot_with_style(slot, val, state, initial_icon_size, initial_is_single);
        let icon = Icon::from_rgba(rgba, initial_icon_size, initial_icon_size)?;
        let tip = slot_tooltip(slot, val, None, &initial_full_tip, initial_is_single);
        let tray = TrayIconBuilder::new()
            .with_id(id)
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(popup.is_none())
            .with_tooltip(tip)
            .with_icon(icon)
            .build()?;
        let visible = slot_visible(slot, &initial_status, &PROFILE);
        if !visible {
            let _ = tray.set_visible(false);
        }
        tray_icons.push((slot, tray, visible));
    }

    if let Err(e) = std::thread::Builder::new()
        .name("tray-promote".to_string())
        .spawn(move || {
            tray_promote::watch_and_promote();
        })
    {
        warn!("Failed to spawn tray-promote thread: {e}");
    }

    let mut current_device_name = device_filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut low_battery_alert = LowBatteryAlert::default();
    let mut current_status = initial_status;
    let mut runtime = RuntimeEstimator::default();
    let mut hotkeys = HotkeyCtl::start();

    let execute_effects =
        |effects: Vec<Effect>,
         cmd_tx: &std::sync::mpsc::Sender<Command>,
         audio_tx: &std::sync::mpsc::Sender<bt_audio::AudioCmd>| {
            for effect in effects {
                match effect {
                    Effect::PauseWorker => {
                        info!("Effect: PauseWorker");
                        let _ = cmd_tx.send(Command::Pause);
                    }
                    Effect::ResumeWorker => {
                        info!("Effect: ResumeWorker");
                        let _ = cmd_tx.send(Command::Resume);
                    }
                    Effect::SendConnect => {
                        info!("Effect: SendConnect");
                        let _ = audio_tx.send(bt_audio::AudioCmd::Connect);
                    }
                    Effect::SendDisconnect => {
                        info!("Effect: SendDisconnect");
                        let _ = audio_tx.send(bt_audio::AudioCmd::Disconnect);
                    }
                }
            }
        };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::Status {
                device_name,
                status,
            }) => {
                if let Some(new_name) =
                    should_update_device(current_device_name.as_deref(), device_name.as_deref())
                {
                    info!("Picked device name updated: {:?}", new_name);
                    let _ = audio_tx.send(bt_audio::AudioCmd::SetDevice(new_name.clone()));
                    current_device_name = Some(new_name);
                }

                let display_name = current_device_name.as_deref().unwrap_or("Earbuds");
                info!(
                    "Event loop received status update for '{}': {:?}",
                    display_name, status
                );
                current_status = status.clone();
                runtime.on_status(Instant::now(), &status);

                let low = low_battery_alert.update(&status);
                if !low.is_empty() {
                    let (toast_title, toast_body) = alert_text(display_name, &low);
                    info!("Low battery alert: {toast_title} - {toast_body}");
                    if let Err(e) = notify::show_toast(&toast_title, &toast_body) {
                        warn!("Failed to show toast notification: {e}");
                    }
                }

                let profile = profile_for(status.source());

                let base_tip = tooltip(display_name, &status);
                let icon_size = tray_icon_size();
                let is_single = is_single_source(status.source());

                for (slot, tray, visible) in &mut tray_icons {
                    let should_be_visible = slot_visible(*slot, &status, &profile);
                    if should_be_visible != *visible {
                        let _ = tray.set_visible(should_be_visible);
                        *visible = should_be_visible;
                    }

                    let (val, state) = slot_value(*slot, &status);
                    let rgba = render_slot_with_style(*slot, val, state, icon_size, is_single);
                    if let Ok(icon) = Icon::from_rgba(rgba, icon_size, icon_size) {
                        let _ = tray.set_icon(Some(icon));
                    }
                }
                let runtime_text = runtime.line(Instant::now(), &status);
                update_tooltips(
                    &mut tray_icons,
                    &status,
                    display_name,
                    link.audio(),
                    runtime_text.as_deref(),
                );

                let menu_line = base_tip.replace('\n', ": ");
                status_item.set_text(menu_line);
                let (toggle_text, toggle_enabled) = link.toggle_label();
                toggle_item.set_text(toggle_text);
                toggle_item.set_enabled(is_toggle_enabled(
                    toggle_enabled,
                    current_device_name.is_some(),
                ));
                refresh_popup(
                    &mut popup,
                    current_device_name.as_deref(),
                    &current_status,
                    &link,
                    &runtime,
                    &hotkeys,
                );
            }
            Event::UserEvent(UserEvent::Audio(audio_event)) => {
                let effects = match audio_event {
                    bt_audio::AudioEvent::State(state) => {
                        info!("Event loop received audio state update: {:?}", state);
                        link.on_audio(state)
                    }
                    bt_audio::AudioEvent::CommandFailed { connect } => {
                        warn!("Audio command failed (connect: {connect})");
                        if !connect {
                            link.on_disconnect_failed()
                        } else {
                            vec![]
                        }
                    }
                };

                audio_item.set_text(audio_line(link.audio()));
                let (toggle_text, toggle_enabled) = link.toggle_label();
                toggle_item.set_text(toggle_text);
                toggle_item.set_enabled(is_toggle_enabled(
                    toggle_enabled,
                    current_device_name.is_some(),
                ));

                let dev_name = current_device_name.as_deref().unwrap_or("Earbuds");
                let runtime_text = runtime.line(Instant::now(), &current_status);
                update_tooltips(
                    &mut tray_icons,
                    &current_status,
                    dev_name,
                    link.audio(),
                    runtime_text.as_deref(),
                );

                refresh_popup(
                    &mut popup,
                    current_device_name.as_deref(),
                    &current_status,
                    &link,
                    &runtime,
                    &hotkeys,
                );
                execute_effects(effects, &cmd_tx, &audio_tx);
            }
            Event::UserEvent(UserEvent::Toggle) => {
                if current_device_name.is_none() {
                    warn!("Toggle requested but no device is selected; ignoring.");
                } else {
                    info!("Toggle requested (current audio: {:?}).", link.audio());
                    let effects = link.toggle();
                    let (toggle_text, toggle_enabled) = link.toggle_label();
                    toggle_item.set_text(toggle_text);
                    toggle_item.set_enabled(is_toggle_enabled(
                        toggle_enabled,
                        current_device_name.is_some(),
                    ));
                    audio_item.set_text(audio_line(link.audio()));
                    refresh_popup(
                        &mut popup,
                        current_device_name.as_deref(),
                        &current_status,
                        &link,
                        &runtime,
                        &hotkeys,
                    );

                    execute_effects(effects, &cmd_tx, &audio_tx);
                }
            }
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            })) => {
                if let Some(p) = popup.as_mut() {
                    match popup_toggle.on_click(Instant::now(), p.is_visible()) {
                        ToggleAction::Show => {
                            p.set_model(full_popup_model(
                                current_device_name.as_deref(),
                                &current_status,
                                &link,
                                &runtime,
                                &hotkeys,
                            ));
                            p.show((position.x.round() as i32, position.y.round() as i32));
                        }
                        ToggleAction::Hide => p.hide(),
                        ToggleAction::Ignore => {}
                    }
                }
            }
            Event::UserEvent(UserEvent::Tray(_)) => {}
            Event::UserEvent(UserEvent::Hotkey(event)) => {
                if let Some(h) = hotkeys.as_mut()
                    && h.on_event(&event, Instant::now())
                {
                    info!("Hotkey pressed.");
                    let _ = proxy_toggle.send_event(UserEvent::Toggle);
                }
            }
            Event::RedrawRequested(window_id)
                if popup.as_ref().is_some_and(|p| p.window_id() == window_id) =>
            {
                if let Some(p) = popup.as_mut() {
                    p.paint();
                }
            }
            Event::WindowEvent {
                window_id, event, ..
            } if popup.as_ref().is_some_and(|p| p.window_id() == window_id) => {
                let Some(p) = popup.as_mut() else { return };
                let recording = hotkeys.as_ref().is_some_and(HotkeyCtl::is_recording);
                match event {
                    WindowEvent::ModifiersChanged(m) => {
                        if let Some(h) = hotkeys.as_mut() {
                            h.mods = Mods {
                                ctrl: m.control_key(),
                                alt: m.alt_key(),
                                shift: m.shift_key(),
                                win: m.super_key(),
                            };
                        }
                    }
                    WindowEvent::KeyboardInput { event: key, .. } if recording => {
                        let Some(h) = hotkeys.as_mut() else { return };
                        if key.state != ElementState::Pressed
                            || key.repeat
                            || is_modifier_key(key.physical_key)
                        {
                            return;
                        }
                        match key.physical_key {
                            KeyCode::Escape => {
                                info!("Hotkey recording cancelled.");
                                h.cancel();
                            }
                            KeyCode::Delete => {
                                info!("Hotkey switched off.");
                                h.turn_off();
                            }
                            _ => {
                                let types_text = h.mods.ctrl
                                    && h.mods.alt
                                    && key.text_with_all_modifiers().is_some_and(|t| {
                                        t.chars().any(|c| !c.is_control() && !c.is_whitespace())
                                    });
                                h.try_keys(recorded_key(&key), types_text);
                            }
                        }
                        p.set_model(full_popup_model(
                            current_device_name.as_deref(),
                            &current_status,
                            &link,
                            &runtime,
                            &hotkeys,
                        ));
                    }
                    WindowEvent::Focused(false) => {
                        if let Some(h) = hotkeys.as_mut() {
                            h.cancel();
                        }
                        if p.is_visible() {
                            p.hide();
                            popup_toggle.on_hidden_by_focus(Instant::now());
                        }
                    }
                    WindowEvent::KeyboardInput { event: key, .. }
                        if key.physical_key == KeyCode::Escape
                            && key.state == ElementState::Pressed =>
                    {
                        p.hide();
                    }
                    WindowEvent::CloseRequested => p.hide(),
                    WindowEvent::CursorMoved { position, .. } => {
                        p.on_cursor_moved(position.x, position.y);
                    }
                    WindowEvent::CursorLeft { .. } => p.on_cursor_left(),
                    WindowEvent::MouseInput {
                        state: ElementState::Released,
                        button: tao::event::MouseButton::Left,
                        ..
                    } => {
                        if p.button_hovered() {
                            info!("Popup: button clicked.");
                            let _ = proxy_toggle.send_event(UserEvent::Toggle);
                        } else if p.change_hovered()
                            && let Some(h) = hotkeys.as_mut()
                        {
                            info!("Popup: hotkey recording started.");
                            h.start_recording();
                            p.set_model(full_popup_model(
                                current_device_name.as_deref(),
                                &current_status,
                                &link,
                                &runtime,
                                &hotkeys,
                            ));
                        }
                    }
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        p.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::UserEvent(UserEvent::Menu(menu_event)) => {
                if menu_event.id == autostart_item.id() {
                    let current_val = autostart::read_run_value();
                    info!(
                        "Tray menu: Start with Windows clicked (currently stored: {current_val:?})."
                    );
                    let is_currently_enabled =
                        autostart::is_enabled_for(current_val.as_deref(), &current_exe);
                    let res = if is_currently_enabled {
                        info!("Disabling autostart for '{current_exe}'");
                        autostart::disable()
                    } else if current_exe.is_empty() {
                        warn!("Not enabling autostart: own exe path unknown");
                        Ok(())
                    } else {
                        info!("Enabling autostart for '{current_exe}'");
                        autostart::enable(&current_exe)
                    };
                    if let Err(e) = res {
                        warn!("Failed to update autostart setting: {e}");
                    }
                    let updated_val = autostart::read_run_value();
                    let new_state = autostart::is_enabled_for(updated_val.as_deref(), &current_exe);
                    autostart_item.set_checked(new_state);
                } else if menu_event.id == toggle_item.id() {
                    info!("Tray menu: Toggle clicked.");
                    let _ = proxy_toggle.send_event(UserEvent::Toggle);
                } else if menu_event.id == refresh_item.id() {
                    info!("Tray menu: Refresh clicked.");
                    let _ = cmd_tx.send(Command::Refresh);
                } else if menu_event.id == quit_item.id() {
                    info!("Tray menu: Quit clicked.");
                    let _ = cmd_tx.send(Command::Quit);
                    let _ = audio_tx.send(bt_audio::AudioCmd::Quit);
                    tray_icons.clear();
                    popup = None;
                    if let Some(h) = hotkeys.as_mut() {
                        h.unregister_active();
                    }
                    hotkeys = None;
                    if let Some(handle) = worker_handle.take() {
                        let _ = handle.join();
                    }
                    if let Some(handle) = audio_handle.take() {
                        let _ = handle.join();
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::LoopDestroyed => {
                let _ = cmd_tx.send(Command::Quit);
                let _ = audio_tx.send(bt_audio::AudioCmd::Quit);
                if let Some(handle) = worker_handle.take() {
                    let _ = handle.join();
                }
                if let Some(handle) = audio_handle.take() {
                    let _ = handle.join();
                }
            }
            _ => {}
        }
    });
}

/// Shows the popup at the mouse position without tray or device and steps
/// through the states every 2 s through the same `set_model` path the tray
/// uses. Opens no channel to any device.
fn run_popup_demo() -> Result<()> {
    use earbuds_tray::app::status::BatteryView;

    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    let mut popup = Popup::new(
        &event_loop,
        popup_model(
            Some("Demo Buds"),
            &DeviceStatus::Connecting,
            ("Connect", false),
            true,
        ),
    )?;
    #[cfg(windows)]
    let click = unsafe {
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        (pt.x, pt.y)
    };
    #[cfg(not(windows))]
    let click = (0, 0);
    popup.show(click);

    let view = |l, r, c| {
        Some(BatteryView {
            left: Some(l),
            right: Some(r),
            case: c,
        })
    };
    let steps: Vec<(DeviceStatus, (&'static str, bool))> = vec![
        (DeviceStatus::Disconnected, ("Disconnect", true)),
        (
            DeviceStatus::Connected {
                view: view(100, 100, Some(100)),
                source: Source::Vendor(Vendor::Edifier),
            },
            ("Disconnect", true),
        ),
        (
            DeviceStatus::Connected {
                view: view(80, 45, Some(15)),
                source: Source::FastPair {
                    model_id: None,
                    case_last_known: true,
                },
            },
            ("Disconnect", true),
        ),
        (
            DeviceStatus::Connected {
                view: view(60, 60, None),
                source: Source::Windows,
            },
            ("Connect", true),
        ),
    ];
    // Phase 12 rows: runtime line and the shortcut row (idle, recording, rejected).
    let vendor_80 = DeviceStatus::Connected {
        view: view(80, 75, Some(60)),
        source: Source::Vendor(Vendor::Edifier),
    };
    let extras: Vec<(DeviceStatus, Option<&str>, ShortcutRow)> = vec![
        (
            vendor_80.clone(),
            Some("~3.5 h left"),
            ShortcutRow::Idle("Ctrl+Alt+B".to_string()),
        ),
        (
            vendor_80.clone(),
            Some("~40 min left"),
            ShortcutRow::Recording(None),
        ),
        (
            vendor_80,
            None,
            ShortcutRow::Recording(Some("Used for typing".to_string())),
        ),
    ];
    let start = Instant::now();
    let mut next = 0usize;
    let mut next_shot = 0usize;
    let shot_dir = std::env::var_os("POPUP_DEMO_SHOTS").map(std::path::PathBuf::from);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
        match event {
            Event::RedrawRequested(id) if id == popup.window_id() => popup.paint(),
            Event::NewEvents(_) => {
                // Screenshot of the popup region one second after each step.
                let shot_due = ((start.elapsed().as_millis() + 1000) / 2000) as usize;
                if shot_due > next_shot {
                    if let Some(dir) = &shot_dir {
                        let (l, t, r, b) = popup.frame_bounds();
                        let path = dir.join(format!("popup-demo-{next_shot}.png"));
                        match capture_screen_png((l - 20, t - 20, r + 20, b + 40), &path) {
                            Ok(()) => info!("popup-demo: saved {}", path.display()),
                            Err(e) => warn!("popup-demo: screenshot failed: {e:#}"),
                        }
                    }
                    next_shot += 1;
                }
                let due = (start.elapsed().as_secs() / 2) as usize;
                if due > next {
                    if let Some((status, toggle)) = steps.get(next) {
                        info!("popup-demo: step {} -> {:?}", next + 1, status);
                        popup.set_model(popup_model(Some("Demo Buds"), status, *toggle, true));
                    } else if let Some((status, runtime, row)) = extras.get(next - steps.len()) {
                        info!("popup-demo: step {} -> {:?} {:?}", next + 1, runtime, row);
                        let mut m =
                            popup_model(Some("Demo Buds"), status, ("Disconnect", true), true);
                        m.runtime = runtime.map(str::to_string);
                        m.shortcut = Some(row.clone());
                        popup.set_model(m);
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                    next += 1;
                }
            }
            _ => {}
        }
    });
}

/// Debug helper for `popup-demo`: saves a screen region as PNG.
#[cfg(windows)]
fn capture_screen_png(rect: (i32, i32, i32, i32), path: &std::path::Path) -> Result<()> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    let (l, t, r, b) = rect;
    let (w, h) = (r - l, b - t);
    anyhow::ensure!(w > 0 && h > 0, "empty capture rect");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        let blt = BitBlt(mem, 0, 0, w, h, screen, l, t, SRCCOPY | CAPTUREBLT);
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        SelectObject(mem, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        blt?;
    }
    for px in buf.chunks_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    let size = resvg::tiny_skia::IntSize::from_wh(w as u32, h as u32)
        .ok_or_else(|| anyhow::anyhow!("bad size"))?;
    let pixmap =
        resvg::tiny_skia::Pixmap::from_vec(buf, size).ok_or_else(|| anyhow::anyhow!("pixmap"))?;
    pixmap.save_png(path)?;
    Ok(())
}

#[cfg(not(windows))]
fn capture_screen_png(_: (i32, i32, i32, i32), _: &std::path::Path) -> Result<()> {
    anyhow::bail!("Windows only")
}

fn run_notify_test(device_filter: &str) -> Result<()> {
    info!("Running notify-test for device '{}'...", device_filter);
    if let Err(e) = notify::register_aumid() {
        warn!("Failed to register AUMID: {e}");
    }

    let (title, body) = alert_text(device_filter, &[(Bud::Left, 14)]);
    info!("Showing test toast: \"{title}\" / \"{body}\"");

    match notify::show_toast(&title, &body) {
        Ok(()) => {
            println!("Toast notification sent: \"{title}\" / \"{body}\"");
        }
        Err(e) => {
            eprintln!("Failed to send toast notification: {e}");
            warn!("Failed to send toast notification: {e}");
        }
    }

    info!("Sleeping 2s for toast delivery before exit...");
    std::thread::sleep(Duration::from_secs(2));
    Ok(())
}

fn run_bt_test(device_filter: &str, action: Option<bool>) -> Result<()> {
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    match action {
        None => {
            info!("Running bt-test for device '{}'...", device_filter);
            match bt_audio::find_device(device_filter)? {
                Some(device) => {
                    println!("Matched device: {}", device.name);
                    println!("Container:      {}", device.container);
                    println!(
                        "Audio state:    {} ({:?})",
                        audio_line(device.state),
                        device.state
                    );
                    println!("Filters ({}):", device.filters.len());
                    for f in &device.filters {
                        println!("  - {f}");
                    }
                }
                None => {
                    println!(
                        "No Bluetooth audio device matched filter '{}'.",
                        device_filter
                    );
                    println!("\nAll Bluetooth audio devices found:");
                    let all = bt_audio::find_all_devices()?;
                    if all.is_empty() {
                        println!("  (none found)");
                    } else {
                        for dev in &all {
                            println!("- Name:        {}", dev.name);
                            println!("  Container:   {}", dev.container);
                            println!("  Audio state: {} ({:?})", audio_line(dev.state), dev.state);
                            println!("  Filters ({}):", dev.filters.len());
                            for f in &dev.filters {
                                println!("    - {f}");
                            }
                        }
                    }
                }
            }
        }
        Some(connect) => {
            let action_name = if connect { "connect" } else { "disconnect" };
            info!(
                "Running bt-test {} for device '{}'...",
                action_name, device_filter
            );

            let device = match bt_audio::find_device(device_filter)? {
                Some(d) => d,
                None => {
                    eprintln!("Device matching '{}' not found.", device_filter);
                    return Ok(());
                }
            };

            println!(
                "Sending {} request to '{}' (container {}, {} filters)...",
                action_name,
                device.name,
                device.container,
                device.filters.len()
            );

            match bt_audio::send(&device, connect) {
                Ok(()) => println!("Send completed successfully."),
                Err(e) => {
                    eprintln!("Send failed: {e}");
                    return Err(e.into());
                }
            }

            println!(
                "Initial state: {} ({:?}). Polling for state changes (up to 15s)...",
                audio_line(device.state),
                device.state
            );

            let start = Instant::now();
            let timeout = Duration::from_secs(15);
            let mut last_state = device.state;

            while start.elapsed() < timeout {
                std::thread::sleep(Duration::from_millis(500));
                match bt_audio::find_device(device_filter)? {
                    Some(current) => {
                        if current.state != last_state {
                            println!(
                                "[+{:.1}s] State changed: {:?} -> {:?}",
                                start.elapsed().as_secs_f32(),
                                last_state,
                                current.state
                            );
                            last_state = current.state;
                        }
                    }
                    None => {
                        if last_state != AudioState::NotFound {
                            println!(
                                "[+{:.1}s] State changed: {:?} -> NotFound",
                                start.elapsed().as_secs_f32(),
                                last_state
                            );
                            last_state = AudioState::NotFound;
                        }
                    }
                }
            }

            println!(
                "Polling finished. Final state: {} ({:?})",
                audio_line(last_state),
                last_state
            );
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let is_console_cmd = args.iter().any(|arg| {
        arg == "probe"
            || arg == "notify-test"
            || arg == "bt-test"
            || arg == "tier-probe"
            || arg == "fp-listen"
            || arg == "popup-demo"
    });

    #[cfg(all(windows, not(debug_assertions)))]
    if is_console_cmd {
        unsafe {
            let _ = windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }

    if cfg!(debug_assertions) || is_console_cmd {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::INFO)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    } else if let Some(file) = logging::open_log_file() {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::INFO)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("Application panic: {info}");
        prev_hook(info);
    }));

    let mut device_filter: Option<String> = None;
    for i in 0..args.len() {
        if (args[i] == "--device" || args[i] == "-d") && i + 1 < args.len() {
            device_filter = Some(args[i + 1].clone());
        }
    }

    #[allow(unused_mut)]
    let mut tier_override = None;
    #[cfg(debug_assertions)]
    for i in 0..args.len() {
        if args[i] == "--tier" && i + 1 < args.len() {
            if args[i + 1].eq_ignore_ascii_case("fastpair") {
                tier_override = Some(earbuds_tray::app::tiers::TierOverride::FastPair);
            } else if args[i + 1].eq_ignore_ascii_case("windows") {
                tier_override = Some(earbuds_tray::app::tiers::TierOverride::Windows);
            }
        }
    }

    if args.iter().any(|arg| arg == "tier-probe") {
        probe::run_tier_probe()?;
    } else if args.iter().any(|arg| arg == "fp-listen") {
        let mut seconds = 60u64;
        for i in 0..args.len() {
            if args[i] == "--seconds"
                && i + 1 < args.len()
                && let Ok(s) = args[i + 1].parse::<u64>()
            {
                seconds = s;
            }
        }
        probe::run_fp_listen(device_filter.as_deref(), seconds)?;
    } else if args.iter().any(|arg| arg == "popup-demo") {
        run_popup_demo()?;
    } else if args.iter().any(|arg| arg == "probe") {
        run_probe(device_filter.as_deref().unwrap_or("NeoBuds Pro 3"))?;
    } else if args.iter().any(|arg| arg == "notify-test") {
        run_notify_test(device_filter.as_deref().unwrap_or("NeoBuds Pro 3"))?;
    } else if args.iter().any(|arg| arg == "bt-test") {
        let action = if args.iter().any(|arg| arg == "disconnect") {
            Some(false)
        } else if args.iter().any(|arg| arg == "connect") {
            Some(true)
        } else {
            None
        };
        run_bt_test(device_filter.as_deref().unwrap_or("NeoBuds Pro 3"), action)?;
    } else {
        run_tray(device_filter, tier_override)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_hex_formatting() {
        let data = [0xAA, 0x01, 0xD0, 0x21, 0x94];
        assert_eq!(to_hex(&data), "AA 01 D0 21 94");
    }

    #[test]
    fn test_slot_tooltip_formatting() {
        let full = "NeoBuds Pro 3\nL 47% | R 89% | Case 52%";
        assert_eq!(
            slot_tooltip(Slot::Left, Some(47), None, full, false),
            "Left 47%\nNeoBuds Pro 3\nL 47% | R 89% | Case 52%"
        );
        assert_eq!(
            slot_tooltip(Slot::Right, Some(89), None, full, false),
            "Right 89%\nNeoBuds Pro 3\nL 47% | R 89% | Case 52%"
        );
        assert_eq!(
            slot_tooltip(Slot::Case, None, None, full, false),
            "Case\nNeoBuds Pro 3\nL 47% | R 89% | Case 52%"
        );
        assert_eq!(
            slot_tooltip(Slot::Left, Some(80), None, "Any Earbuds: 80%", true),
            "Battery 80%\nAny Earbuds: 80%"
        );
    }

    #[test]
    fn test_slot_tooltip_runtime_on_second_line() {
        let full = "NeoBuds Pro 3
L 47% | R 89%
Audio: connected";
        assert_eq!(
            slot_tooltip(Slot::Left, Some(47), Some("~3 h left"), full, false),
            "Left 47%
~3 h left
NeoBuds Pro 3
L 47% | R 89%
Audio: connected"
        );
    }

    #[test]
    fn test_should_update_device_when_none_to_some() {
        assert_eq!(
            should_update_device(None, Some("EDIFIER NeoBuds Pro 3")),
            Some("EDIFIER NeoBuds Pro 3".to_string())
        );
    }

    #[test]
    fn test_should_update_device_when_same() {
        assert_eq!(
            should_update_device(Some("EDIFIER NeoBuds Pro 3"), Some("EDIFIER NeoBuds Pro 3")),
            None
        );
    }

    #[test]
    fn test_should_update_device_when_different() {
        assert_eq!(
            should_update_device(Some("Old Buds"), Some("New Buds")),
            Some("New Buds".to_string())
        );
    }

    #[test]
    fn test_should_update_device_when_new_is_none() {
        assert_eq!(should_update_device(None, None), None);
        assert_eq!(should_update_device(Some("Buds"), None), None);
    }

    #[test]
    fn test_should_update_device_when_empty_or_whitespace() {
        assert_eq!(should_update_device(None, Some("")), None);
        assert_eq!(should_update_device(None, Some("   \t  ")), None);
        assert_eq!(should_update_device(Some("Buds"), Some("   ")), None);
    }

    #[test]
    fn test_is_toggle_enabled() {
        assert!(is_toggle_enabled(true, true));
        assert!(!is_toggle_enabled(true, false));
        assert!(!is_toggle_enabled(false, true));
        assert!(!is_toggle_enabled(false, false));
    }

    fn galaxy_status(case: Option<u8>) -> DeviceStatus {
        DeviceStatus::Connected {
            view: Some(earbuds_tray::app::status::BatteryView {
                left: Some(70),
                right: Some(95),
                case,
            }),
            source: Source::Vendor(Vendor::Galaxy),
        }
    }

    #[test]
    fn test_galaxy_case_slot_only_with_value() {
        let with = galaxy_status(Some(88));
        let without = galaxy_status(None);
        let p = profile_for(with.source());
        assert!(slot_visible(Slot::Left, &with, &p));
        assert!(slot_visible(Slot::Right, &with, &p));
        assert!(slot_visible(Slot::Case, &with, &p));
        let p = profile_for(without.source());
        assert!(slot_visible(Slot::Left, &without, &p));
        assert!(!slot_visible(Slot::Case, &without, &p));
    }

    #[test]
    fn test_vendor_hfp_is_single_bar_without_case() {
        let src = Some(Source::VendorHfp(Vendor::Galaxy));
        assert!(is_single_source(src));
        let status = DeviceStatus::Connected {
            view: Some(earbuds_tray::app::status::BatteryView {
                left: Some(60),
                right: None,
                case: None,
            }),
            source: Source::VendorHfp(Vendor::Galaxy),
        };
        let p = profile_for(src);
        assert!(slot_visible(Slot::Left, &status, &p));
        assert!(!slot_visible(Slot::Right, &status, &p));
        assert!(!slot_visible(Slot::Case, &status, &p));
    }

    #[test]
    fn test_edifier_profile_and_single_unchanged() {
        let src = Some(Source::Vendor(Vendor::Edifier));
        assert!(!is_single_source(src));
        assert_eq!(profile_for(src), PROFILE);
        assert!(is_single_source(Some(Source::Windows)));
        assert!(is_single_source(Some(Source::FastPairHfp)));
    }
}
