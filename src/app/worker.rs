use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

use super::discovery;
use super::session::Session;
use super::status::{BatteryView, DeviceStatus, FastPairBatteryTracker, Source};
use super::tiers::{
    GALAXY_LEGACY_SERVICE_UUID, GALAXY_NEW_SERVICE_UUID, Tier, TierOverride, Vendor,
    galaxy_service, pick,
};
use crate::drivers::edifier::query::{Format, Query};
use crate::drivers::edifier::transport::EdifierTransport;
use crate::drivers::fastpair::FastPairTransport;
use crate::drivers::fastpair::message::{FrameParser, Message};
use crate::drivers::galaxy::GalaxyTransport;
use crate::drivers::galaxy::message::{
    FrameParser as GalaxyFrameParser, Framing, Message as GalaxyMessage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Refresh,
    Quit,
    Pause,
    Resume,
}

pub fn resolve_fallback_device(
    override_filter: Option<&str>,
    last_device: Option<&str>,
) -> Option<String> {
    override_filter
        .or(last_device)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub fn spawn(
    override_filter: Option<String>,
    tier_override: Option<TierOverride>,
    on_status: impl Fn(Option<String>, DeviceStatus) + Send + 'static,
    commands: Receiver<Command>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        worker_loop(
            override_filter.as_deref(),
            tier_override,
            on_status,
            commands,
        );
    })
}

fn wait_paused(commands: &Receiver<Command>) -> bool {
    loop {
        match commands.recv() {
            Ok(Command::Resume) => {
                info!("Worker received Resume command. Resuming connection.");
                return true;
            }
            Ok(Command::Quit) => {
                info!("Worker received Quit command while paused. Exiting.");
                return false;
            }
            Ok(Command::Refresh) => {
                info!("Worker received Refresh command while paused. Ignoring.");
            }
            Ok(Command::Pause) => {
                info!("Worker received Pause command while already paused. Ignoring.");
            }
            Err(_) => {
                info!("Worker command channel disconnected while paused. Exiting.");
                return false;
            }
        }
    }
}

fn sleep_or_cmd(commands: &Receiver<Command>, duration: Duration) -> Result<Option<Command>, ()> {
    let start = Instant::now();
    while start.elapsed() < duration {
        let remaining = duration.saturating_sub(start.elapsed());
        let step = remaining.min(Duration::from_millis(200));
        match commands.recv_timeout(step) {
            Ok(cmd) => return Ok(Some(cmd)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(()),
        }
    }
    Ok(None)
}

fn worker_loop(
    override_filter: Option<&str>,
    tier_override: Option<TierOverride>,
    on_status: impl Fn(Option<String>, DeviceStatus),
    commands: Receiver<Command>,
) {
    let mut backoff = Duration::from_secs(5);
    const MAX_BACKOFF: Duration = Duration::from_secs(60);

    'outer: loop {
        match commands.try_recv() {
            Ok(Command::Quit) => {
                info!("Worker received Quit command before discovery. Exiting.");
                break 'outer;
            }
            Ok(Command::Pause) => {
                info!("Worker received Pause command before discovery. Waiting for Resume...");
                if !wait_paused(&commands) {
                    break 'outer;
                }
                backoff = Duration::from_secs(5);
                continue 'outer;
            }
            Ok(Command::Resume) => {
                info!("Worker received Resume command while not paused. Ignoring.");
            }
            Ok(Command::Refresh) => {}
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                info!("Worker command channel disconnected. Exiting.");
                break 'outer;
            }
        }

        let candidates = match discovery::candidates(tier_override) {
            Ok(c) => c,
            Err(e) => {
                warn!("Candidate discovery failed: {e}");
                Vec::new()
            }
        };

        let last_device = discovery::read_last_device();
        let picked = pick(&candidates, override_filter, last_device.as_deref()).cloned();

        let candidate = match picked {
            Some(c) => {
                info!(
                    "Picked candidate '{}' (Tier: {:?}, Connected: {})",
                    c.name, c.tier, c.connected
                );
                if last_device.as_deref() != Some(&c.name) {
                    let _ = discovery::write_last_device(&c.name);
                }
                c
            }
            None => {
                let real_name = resolve_fallback_device(override_filter, last_device.as_deref());
                let display_name = real_name.as_deref().unwrap_or("Earbuds");
                warn!(
                    "No candidate Bluetooth device picked. Emitting Disconnected for '{display_name}'. Backing off for {}s...",
                    backoff.as_secs()
                );
                on_status(real_name, DeviceStatus::Disconnected);

                match sleep_or_cmd(&commands, backoff) {
                    Ok(Some(Command::Quit)) | Err(()) => break 'outer,
                    Ok(Some(Command::Pause)) => {
                        if !wait_paused(&commands) {
                            break 'outer;
                        }
                        backoff = Duration::from_secs(5);
                        continue 'outer;
                    }
                    Ok(Some(Command::Refresh)) => {
                        info!("Refresh requested during backoff; retrying discovery immediately.");
                    }
                    Ok(Some(Command::Resume)) | Ok(None) => {}
                }

                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue 'outer;
            }
        };

        let device_name = candidate.name.clone();

        match candidate.tier {
            Tier::Vendor(Vendor::Edifier) => {
                info!(
                    "Attempting connection to '{}' via Edifier transport...",
                    device_name
                );
                on_status(Some(device_name.clone()), DeviceStatus::Connecting);

                let transport = match EdifierTransport::connect(&device_name) {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(
                            "Edifier connection failed for '{}': {e}. Emitting Disconnected. Backing off for {}s...",
                            device_name,
                            backoff.as_secs()
                        );
                        on_status(Some(device_name.clone()), DeviceStatus::Disconnected);

                        match sleep_or_cmd(&commands, backoff) {
                            Ok(Some(Command::Quit)) | Err(()) => break 'outer,
                            Ok(Some(Command::Pause)) => {
                                if !wait_paused(&commands) {
                                    break 'outer;
                                }
                                backoff = Duration::from_secs(5);
                                continue 'outer;
                            }
                            Ok(Some(Command::Refresh)) => {
                                info!(
                                    "Refresh requested during backoff; retrying connection immediately."
                                );
                            }
                            Ok(Some(Command::Resume)) | Ok(None) => {}
                        }

                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue 'outer;
                    }
                };

                info!(
                    "Connected to '{}' (Vendor Edifier). Resetting backoff.",
                    device_name
                );
                on_status(
                    Some(device_name.clone()),
                    DeviceStatus::connected_vendor(None),
                );
                backoff = Duration::from_secs(5);

                if let Err(e) = transport.send_query(Query::BatteryF2, Format::V2Xor) {
                    warn!("Failed to send initial F2 query: {e}");
                }

                let mut session = Session::new();
                let mut last_f2_activity = Instant::now();

                loop {
                    loop {
                        match commands.try_recv() {
                            Ok(Command::Quit) => {
                                info!("Worker received Quit command while connected. Exiting.");
                                break 'outer;
                            }
                            Ok(Command::Refresh) => {
                                info!("Worker received Refresh command. Sending F2 query...");
                                if let Err(e) =
                                    transport.send_query(Query::BatteryF2, Format::V2Xor)
                                {
                                    warn!("Failed to send F2 query on Refresh: {e}");
                                }
                                last_f2_activity = Instant::now();
                            }
                            Ok(Command::Pause) => {
                                info!(
                                    "Worker received Pause command while connected. Dropping transport and emitting Disconnected."
                                );
                                drop(transport);
                                on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                                if !wait_paused(&commands) {
                                    break 'outer;
                                }
                                backoff = Duration::from_secs(5);
                                continue 'outer;
                            }
                            Ok(Command::Resume) => {
                                info!("Worker received Resume command while not paused. Ignoring.");
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                info!("Worker command channel disconnected. Exiting.");
                                break 'outer;
                            }
                        }
                    }

                    match transport.rx().recv_timeout(Duration::from_secs(1)) {
                        Ok(chunk) => {
                            if let Some(view) = session.push(&chunk) {
                                info!("Received updated battery view: {:?}", view);
                                on_status(
                                    Some(device_name.clone()),
                                    DeviceStatus::connected_vendor(Some(view)),
                                );
                                last_f2_activity = Instant::now();
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if last_f2_activity.elapsed() >= Duration::from_secs(60) {
                                info!(
                                    "60s elapsed without push. Sending periodic F2 query fallback..."
                                );
                                if let Err(e) =
                                    transport.send_query(Query::BatteryF2, Format::V2Xor)
                                {
                                    warn!("Failed to send periodic F2 query: {e}");
                                }
                                last_f2_activity = Instant::now();
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            info!("Bluetooth transport disconnected (lid closed or out of range).");
                            on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                            break;
                        }
                    }
                }
            }

            Tier::Vendor(Vendor::Galaxy) => {
                info!(
                    "Attempting connection to '{}' via Galaxy transport...",
                    device_name
                );
                on_status(Some(device_name.clone()), DeviceStatus::Connecting);

                let galaxy_uuid =
                    galaxy_service(&candidate.services).unwrap_or(GALAXY_NEW_SERVICE_UUID);
                let framing = if galaxy_uuid == GALAXY_LEGACY_SERVICE_UUID {
                    Framing::Legacy
                } else {
                    Framing::New
                };
                let transport = match GalaxyTransport::connect(&device_name, galaxy_uuid) {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(
                            "Galaxy connection failed for '{}': {e}. Emitting Disconnected. Backing off for {}s...",
                            device_name,
                            backoff.as_secs()
                        );
                        on_status(Some(device_name.clone()), DeviceStatus::Disconnected);

                        match sleep_or_cmd(&commands, backoff) {
                            Ok(Some(Command::Quit)) | Err(()) => break 'outer,
                            Ok(Some(Command::Pause)) => {
                                if !wait_paused(&commands) {
                                    break 'outer;
                                }
                                backoff = Duration::from_secs(5);
                                continue 'outer;
                            }
                            Ok(Some(Command::Refresh)) => {
                                info!(
                                    "Refresh requested during backoff; retrying connection immediately."
                                );
                            }
                            Ok(Some(Command::Resume)) | Ok(None) => {}
                        }

                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue 'outer;
                    }
                };

                info!(
                    "Connected to '{}' (Galaxy listen-only, beta). Resetting backoff.",
                    device_name
                );
                backoff = Duration::from_secs(5);

                let init_hfp = discovery::read_hfp_battery(&candidate.container);
                let init_view = init_hfp.map(|pct| BatteryView {
                    left: Some(pct),
                    right: None,
                    case: None,
                });
                on_status(
                    Some(device_name.clone()),
                    DeviceStatus::Connected {
                        view: init_view,
                        source: Source::VendorHfp(Vendor::Galaxy),
                    },
                );

                let mut has_received_status = false;
                let mut parser = GalaxyFrameParser::new(framing);
                let mut last_hfp_poll = Instant::now();

                loop {
                    loop {
                        match commands.try_recv() {
                            Ok(Command::Quit) => {
                                info!(
                                    "Worker received Quit command while on Galaxy stream. Exiting."
                                );
                                break 'outer;
                            }
                            Ok(Command::Refresh) => {
                                if !has_received_status {
                                    info!(
                                        "Worker received Refresh on Galaxy before frame. Querying HFP battery property..."
                                    );
                                    let hfp = discovery::read_hfp_battery(&candidate.container);
                                    let view = hfp.map(|pct| BatteryView {
                                        left: Some(pct),
                                        right: None,
                                        case: None,
                                    });
                                    on_status(
                                        Some(device_name.clone()),
                                        DeviceStatus::Connected {
                                            view,
                                            source: Source::VendorHfp(Vendor::Galaxy),
                                        },
                                    );
                                    last_hfp_poll = Instant::now();
                                } else {
                                    info!(
                                        "Worker received Refresh on Galaxy channel (listen-only; skipping send)."
                                    );
                                }
                            }
                            Ok(Command::Pause) => {
                                info!(
                                    "Worker received Pause command while on Galaxy. Dropping transport and emitting Disconnected."
                                );
                                drop(transport);
                                on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                                if !wait_paused(&commands) {
                                    break 'outer;
                                }
                                backoff = Duration::from_secs(5);
                                continue 'outer;
                            }
                            Ok(Command::Resume) => {
                                info!("Worker received Resume command while not paused. Ignoring.");
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                info!("Worker command channel disconnected. Exiting.");
                                break 'outer;
                            }
                        }
                    }

                    match transport.rx().recv_timeout(Duration::from_millis(500)) {
                        Ok(chunk) => {
                            let messages = parser.push(&chunk);
                            for msg in messages {
                                match msg {
                                    GalaxyMessage::Status { left, right, case } => {
                                        info!(
                                            "Galaxy status: L {:?}, R {:?}, Case {:?}",
                                            left, right, case
                                        );
                                        if left.is_none() && right.is_none() {
                                            continue;
                                        }
                                        has_received_status = true;
                                        on_status(
                                            Some(device_name.clone()),
                                            DeviceStatus::Connected {
                                                view: Some(BatteryView { left, right, case }),
                                                source: Source::Vendor(Vendor::Galaxy),
                                            },
                                        );
                                    }
                                    GalaxyMessage::Other(id) => {
                                        tracing::debug!("Galaxy frame ignored: id 0x{:02X}", id);
                                    }
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if !has_received_status
                                && last_hfp_poll.elapsed() >= Duration::from_secs(60)
                            {
                                info!(
                                    "60s elapsed without Galaxy frame. Refreshing HFP battery property..."
                                );
                                let hfp = discovery::read_hfp_battery(&candidate.container);
                                let view = hfp.map(|pct| BatteryView {
                                    left: Some(pct),
                                    right: None,
                                    case: None,
                                });
                                on_status(
                                    Some(device_name.clone()),
                                    DeviceStatus::Connected {
                                        view,
                                        source: Source::VendorHfp(Vendor::Galaxy),
                                    },
                                );
                                last_hfp_poll = Instant::now();
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            info!("Galaxy transport disconnected by remote peer.");
                            on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                            break;
                        }
                    }
                }
            }

            Tier::FastPair => {
                info!(
                    "Attempting connection to '{}' via Fast Pair transport...",
                    device_name
                );
                on_status(Some(device_name.clone()), DeviceStatus::Connecting);

                let transport = match FastPairTransport::connect(&device_name) {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(
                            "Fast Pair connection failed for '{}': {e}. Emitting Disconnected. Backing off for {}s...",
                            device_name,
                            backoff.as_secs()
                        );
                        on_status(Some(device_name.clone()), DeviceStatus::Disconnected);

                        match sleep_or_cmd(&commands, backoff) {
                            Ok(Some(Command::Quit)) | Err(()) => break 'outer,
                            Ok(Some(Command::Pause)) => {
                                if !wait_paused(&commands) {
                                    break 'outer;
                                }
                                backoff = Duration::from_secs(5);
                                continue 'outer;
                            }
                            Ok(Some(Command::Refresh)) => {
                                info!(
                                    "Refresh requested during backoff; retrying connection immediately."
                                );
                            }
                            Ok(Some(Command::Resume)) | Ok(None) => {}
                        }

                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue 'outer;
                    }
                };

                info!(
                    "Connected to '{}' (Fast Pair listen-only). Resetting backoff.",
                    device_name
                );
                backoff = Duration::from_secs(5);

                let init_hfp = discovery::read_hfp_battery(&candidate.container);
                let init_view = init_hfp.map(|pct| BatteryView {
                    left: Some(pct),
                    right: None,
                    case: None,
                });
                on_status(
                    Some(device_name.clone()),
                    DeviceStatus::Connected {
                        view: init_view,
                        source: Source::FastPairHfp,
                    },
                );

                let mut has_received_fp_battery = false;
                let mut model_id: Option<[u8; 3]> = None;
                let mut parser = FrameParser::new();
                let mut tracker = FastPairBatteryTracker::new();
                let mut last_hfp_poll = Instant::now();

                loop {
                    loop {
                        match commands.try_recv() {
                            Ok(Command::Quit) => {
                                info!(
                                    "Worker received Quit command while on Fast Pair stream. Exiting."
                                );
                                break 'outer;
                            }
                            Ok(Command::Refresh) => {
                                if !has_received_fp_battery {
                                    info!(
                                        "Worker received Refresh on Fast Pair before frame. Querying HFP battery property..."
                                    );
                                    let hfp = discovery::read_hfp_battery(&candidate.container);
                                    let view = hfp.map(|pct| BatteryView {
                                        left: Some(pct),
                                        right: None,
                                        case: None,
                                    });
                                    on_status(
                                        Some(device_name.clone()),
                                        DeviceStatus::Connected {
                                            view,
                                            source: Source::FastPairHfp,
                                        },
                                    );
                                    last_hfp_poll = Instant::now();
                                } else {
                                    info!(
                                        "Worker received Refresh on Fast Pair channel (listen-only; skipping send)."
                                    );
                                }
                            }
                            Ok(Command::Pause) => {
                                info!(
                                    "Worker received Pause command while on Fast Pair. Dropping transport and emitting Disconnected."
                                );
                                drop(transport);
                                tracker.reset();
                                on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                                if !wait_paused(&commands) {
                                    break 'outer;
                                }
                                backoff = Duration::from_secs(5);
                                continue 'outer;
                            }
                            Ok(Command::Resume) => {
                                info!("Worker received Resume command while not paused. Ignoring.");
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                info!("Worker command channel disconnected. Exiting.");
                                break 'outer;
                            }
                        }
                    }

                    match transport.rx().recv_timeout(Duration::from_millis(500)) {
                        Ok(chunk) => {
                            let messages = parser.push(&chunk);
                            for msg in messages {
                                match msg {
                                    Message::ModelId(id) => {
                                        info!(
                                            "Fast Pair Model ID: {:02X}{:02X}{:02X}",
                                            id[0], id[1], id[2]
                                        );
                                        model_id = Some(id);
                                    }
                                    Message::Battery { left, right, case } => {
                                        info!(
                                            "Fast Pair Battery frame: L {:?}, R {:?}, Case {:?}",
                                            left, right, case
                                        );
                                        has_received_fp_battery = true;
                                        let status = tracker.status_for(
                                            left.percent,
                                            right.percent,
                                            case.percent,
                                            model_id,
                                        );
                                        on_status(Some(device_name.clone()), status);
                                    }
                                    Message::Other { group, code } => {
                                        tracing::debug!(
                                            "Fast Pair frame ignored: group 0x{:02X}, code 0x{:02X}",
                                            group,
                                            code
                                        );
                                    }
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if !has_received_fp_battery
                                && last_hfp_poll.elapsed() >= Duration::from_secs(60)
                            {
                                info!(
                                    "60s elapsed without Fast Pair frame. Refreshing HFP battery property..."
                                );
                                let hfp = discovery::read_hfp_battery(&candidate.container);
                                let view = hfp.map(|pct| BatteryView {
                                    left: Some(pct),
                                    right: None,
                                    case: None,
                                });
                                on_status(
                                    Some(device_name.clone()),
                                    DeviceStatus::Connected {
                                        view,
                                        source: Source::FastPairHfp,
                                    },
                                );
                                last_hfp_poll = Instant::now();
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            info!("Fast Pair transport disconnected by remote peer.");
                            tracker.reset();
                            on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                            break;
                        }
                    }
                }
            }

            Tier::Windows => {
                info!("Polling Windows HFP battery for '{}'...", device_name);
                if !candidate.connected && !discovery::is_device_connected(&device_name) {
                    warn!(
                        "Device '{}' is not connected. Emitting Disconnected. Backing off for {}s...",
                        device_name,
                        backoff.as_secs()
                    );
                    on_status(Some(device_name.clone()), DeviceStatus::Disconnected);

                    match sleep_or_cmd(&commands, backoff) {
                        Ok(Some(Command::Quit)) | Err(()) => break 'outer,
                        Ok(Some(Command::Pause)) => {
                            if !wait_paused(&commands) {
                                break 'outer;
                            }
                            backoff = Duration::from_secs(5);
                            continue 'outer;
                        }
                        Ok(Some(Command::Refresh)) => {
                            info!("Refresh requested during backoff; retrying immediately.");
                        }
                        Ok(Some(Command::Resume)) | Ok(None) => {}
                    }

                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    continue 'outer;
                }

                backoff = Duration::from_secs(5);
                let hfp = discovery::read_hfp_battery(&candidate.container);
                let view = hfp.map(|pct| BatteryView {
                    left: Some(pct),
                    right: None,
                    case: None,
                });
                on_status(
                    Some(device_name.clone()),
                    DeviceStatus::Connected {
                        view,
                        source: Source::Windows,
                    },
                );

                let mut last_poll = Instant::now();

                loop {
                    match commands.recv_timeout(Duration::from_secs(1)) {
                        Ok(Command::Quit) => {
                            info!("Worker received Quit command while in Windows tier. Exiting.");
                            break 'outer;
                        }
                        Ok(Command::Pause) => {
                            info!(
                                "Worker received Pause command while in Windows tier. Emitting Disconnected."
                            );
                            on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                            if !wait_paused(&commands) {
                                break 'outer;
                            }
                            backoff = Duration::from_secs(5);
                            continue 'outer;
                        }
                        Ok(Command::Refresh) => {
                            info!(
                                "Worker received Refresh command in Windows tier. Polling property..."
                            );
                            if !discovery::is_device_connected(&device_name) {
                                info!("Device '{}' disconnected.", device_name);
                                on_status(Some(device_name.clone()), DeviceStatus::Disconnected);
                                break;
                            }
                            let hfp = discovery::read_hfp_battery(&candidate.container);
                            let view = hfp.map(|pct| BatteryView {
                                left: Some(pct),
                                right: None,
                                case: None,
                            });
                            on_status(
                                Some(device_name.clone()),
                                DeviceStatus::Connected {
                                    view,
                                    source: Source::Windows,
                                },
                            );
                            last_poll = Instant::now();
                        }
                        Ok(Command::Resume) => {
                            info!("Worker received Resume command while not paused. Ignoring.");
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            info!("Worker command channel disconnected. Exiting.");
                            break 'outer;
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if last_poll.elapsed() >= Duration::from_secs(60) {
                                if !discovery::is_device_connected(&device_name) {
                                    info!(
                                        "Device '{}' disconnected during periodic check.",
                                        device_name
                                    );
                                    on_status(
                                        Some(device_name.clone()),
                                        DeviceStatus::Disconnected,
                                    );
                                    break;
                                }
                                let hfp = discovery::read_hfp_battery(&candidate.container);
                                let view = hfp.map(|pct| BatteryView {
                                    left: Some(pct),
                                    right: None,
                                    case: None,
                                });
                                on_status(
                                    Some(device_name.clone()),
                                    DeviceStatus::Connected {
                                        view,
                                        source: Source::Windows,
                                    },
                                );
                                last_poll = Instant::now();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_fallback_device_with_override() {
        assert_eq!(
            resolve_fallback_device(Some("EDIFIER NeoBuds Pro 3"), None),
            Some("EDIFIER NeoBuds Pro 3".to_string())
        );
    }

    #[test]
    fn test_resolve_fallback_device_with_last_device() {
        assert_eq!(
            resolve_fallback_device(None, Some("Pixel Buds Pro")),
            Some("Pixel Buds Pro".to_string())
        );
    }

    #[test]
    fn test_resolve_fallback_device_override_takes_precedence() {
        assert_eq!(
            resolve_fallback_device(Some("Override Buds"), Some("Cached Buds")),
            Some("Override Buds".to_string())
        );
    }

    #[test]
    fn test_resolve_fallback_device_none_when_both_missing() {
        assert_eq!(resolve_fallback_device(None, None), None);
    }

    #[test]
    fn test_resolve_fallback_device_ignores_whitespace_only() {
        assert_eq!(resolve_fallback_device(Some("   "), None), None);
        assert_eq!(resolve_fallback_device(None, Some("  \t\n ")), None);
        assert_eq!(resolve_fallback_device(Some("   "), Some("  \t\n ")), None);
    }

    #[test]
    fn test_resolve_fallback_device_trims_whitespace() {
        assert_eq!(
            resolve_fallback_device(Some("  NeoBuds  "), None),
            Some("NeoBuds".to_string())
        );
    }
}
