use anyhow::{Context, Result, bail};
use std::time::{Duration, Instant};
use tracing::{info, warn};

pub const FAST_PAIR_SERVICE_UUID: windows::core::GUID =
    windows::core::GUID::from_u128(0xDF21FE2C_2515_4FDB_8886_F12C4D67927C);

pub fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
pub fn run_tier_probe() -> Result<()> {
    use windows::Devices::Bluetooth::{
        BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice,
    };
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Devices::Enumeration::Pnp::{PnpObject, PnpObjectType};
    use windows::Foundation::Collections::IIterable;
    use windows::Foundation::{IPropertyValue, PropertyType};
    use windows::core::{HSTRING, Interface};

    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    println!("============================================================");
    println!("               EARBUDS-TRAY: TIER PROBE");
    println!("============================================================\n");

    let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    let props: IIterable<HSTRING> =
        IIterable::try_from(vec![HSTRING::from("System.Devices.Aep.ContainerId")])?;
    let paired_devices =
        DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties(&selector, &props)?
            .get()
            .context("Failed to enumerate paired devices")?;

    struct PairedInfo {
        name: String,
        aep_id: String,
        container_id: String,
        connected: bool,
        rfcomm_uuids: Vec<String>,
        hfp_battery: Option<u8>,
    }

    let mut paired_list: Vec<PairedInfo> = Vec::new();

    for dev in &paired_devices {
        let name = dev.Name().unwrap_or_default().to_string();
        let aep_id = dev.Id().unwrap_or_default().to_string();

        let container_id = dev
            .Properties()
            .ok()
            .and_then(|props| {
                props
                    .Lookup(&HSTRING::from("System.Devices.Aep.ContainerId"))
                    .ok()
            })
            .and_then(|val| val.cast::<IPropertyValue>().ok())
            .map(|pv| match pv.Type().unwrap_or(PropertyType::Empty) {
                PropertyType::Guid => format!("{{{:?}}}", pv.GetGuid().unwrap_or_default()),
                PropertyType::String => pv.GetString().unwrap_or_default().to_string(),
                _ => String::new(),
            })
            .unwrap_or_default();

        let mut connected = false;
        let mut rfcomm_uuids = Vec::new();

        if let Ok(async_op) = BluetoothDevice::FromIdAsync(&HSTRING::from(&aep_id))
            && let Ok(bt_device) = async_op.get()
        {
            connected = bt_device
                .ConnectionStatus()
                .unwrap_or(BluetoothConnectionStatus::Disconnected)
                == BluetoothConnectionStatus::Connected;

            if let Ok(services_op) =
                bt_device.GetRfcommServicesWithCacheModeAsync(BluetoothCacheMode::Cached)
                && let Ok(services_res) = services_op.get()
                && let Ok(services) = services_res.Services()
            {
                for s in &services {
                    if let Ok(s_id) = s.ServiceId()
                        && let Ok(uuid) = s_id.Uuid()
                    {
                        rfcomm_uuids.push(format!("{{{:?}}}", uuid));
                    }
                }
            }
        }

        // HFP battery lookup via PnpObject
        let mut hfp_battery = None;
        if !container_id.is_empty() {
            let pnp_prop_name = HSTRING::from("{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2");
            let pnp_props: IIterable<HSTRING> = IIterable::try_from(vec![pnp_prop_name.clone()])?;
            let aqs = format!(r#"System.Devices.ContainerId:="{}""#, container_id);
            if let Ok(pnp_op) = PnpObject::FindAllAsyncAqsFilter(
                PnpObjectType::Device,
                &pnp_props,
                &HSTRING::from(aqs),
            ) && let Ok(pnp_objs) = pnp_op.get()
            {
                for pnp_obj in &pnp_objs {
                    if let Ok(props) = pnp_obj.Properties()
                        && let Ok(val) = props.Lookup(&pnp_prop_name)
                        && let Ok(pv) = val.cast::<IPropertyValue>()
                    {
                        let pct = match pv.Type().unwrap_or(PropertyType::Empty) {
                            PropertyType::UInt8 => pv.GetUInt8().ok(),
                            PropertyType::Int32 => pv.GetInt32().ok().map(|v| v as u8),
                            PropertyType::UInt32 => pv.GetUInt32().ok().map(|v| v as u8),
                            _ => None,
                        };
                        if let Some(p) = pct {
                            hfp_battery = Some(p);
                            break;
                        }
                    }
                }
            }
        }

        paired_list.push(PairedInfo {
            name,
            aep_id,
            container_id,
            connected,
            rfcomm_uuids,
            hfp_battery,
        });
    }

    println!(
        "--- 1. PAIRED BLUETOOTH DEVICES ({} found) ---",
        paired_list.len()
    );
    for (i, p) in paired_list.iter().enumerate() {
        println!("[{}] {}", i + 1, p.name);
        println!(
            "    Connected:    {}",
            if p.connected { "YES" } else { "NO" }
        );
        println!("    AEP ID:       {}", p.aep_id);
        println!(
            "    Container ID: {}",
            if p.container_id.is_empty() {
                "(none)"
            } else {
                &p.container_id
            }
        );
        println!(
            "    HFP Battery:  {}",
            match p.hfp_battery {
                Some(pct) => format!("{}%", pct),
                None => "(none / not reported)".to_string(),
            }
        );
        println!("    Cached RFCOMM Services ({}):", p.rfcomm_uuids.len());
        for uuid in &p.rfcomm_uuids {
            let note = if uuid
                .to_ascii_uppercase()
                .contains("EDF00000-EDFE-DFED-FEDF-EDFEDFEDFEDF")
            {
                " <- Edifier Vendor Service"
            } else if uuid
                .to_ascii_uppercase()
                .contains("DF21FE2C-2515-4FDB-8886-F12C4D67927C")
            {
                " <- Fast Pair Message Stream"
            } else if uuid
                .to_ascii_uppercase()
                .contains("2E73A4AD-332D-41FC-90E2-16BEF06523F2")
            {
                " <- Galaxy Buds+ and newer (listen only, beta)"
            } else if uuid
                .to_ascii_uppercase()
                .contains("00001102-0000-1000-8000-00805F9B34FD")
            {
                " <- Galaxy Buds legacy (listen only, beta)"
            } else {
                ""
            };
            println!("      - {}{}", uuid, note);
        }
        println!();
    }

    // Audio endpoints from bt_audio
    println!("--- 2. AUDIO CONTAINERS (bt_audio::find_all_devices) ---");
    let audio_devices = crate::app::bt_audio::find_all_devices()?;
    if audio_devices.is_empty() {
        println!("  (no Bluetooth audio devices found)");
    } else {
        for (i, dev) in audio_devices.iter().enumerate() {
            let paired_match = paired_list.iter().find(|p| {
                !p.container_id.is_empty()
                    && p.container_id
                        .trim_matches(['{', '}'])
                        .eq_ignore_ascii_case(dev.container.trim_matches(['{', '}']))
            });
            let match_str = match paired_match {
                Some(p) => format!(
                    "YES -> '{}' (Connected: {})",
                    p.name,
                    if p.connected { "YES" } else { "NO" }
                ),
                None => "NO (no paired BluetoothDevice has this container ID)".to_string(),
            };
            println!("[{}] Name:      {}", i + 1, dev.name);
            println!("    Container: {}", dev.container);
            println!("    State:     {:?}", dev.state);
            println!("    Filters:   {}", dev.filters.len());
            println!("    Maps to Paired Device: {}", match_str);
            println!();
        }
    }

    println!("============================================================");
    println!("Probe complete.");
    println!("============================================================");
    Ok(())
}

#[cfg(not(windows))]
pub fn run_tier_probe() -> Result<()> {
    println!("tier-probe is only supported on Windows.");
    Ok(())
}

#[cfg(windows)]
pub fn run_fp_listen(device_filter: Option<&str>, seconds: u64) -> Result<()> {
    use crate::drivers::fastpair::message::{FrameParser, Message};
    use windows::Devices::Bluetooth::BluetoothDevice;
    use windows::Devices::Bluetooth::Rfcomm::RfcommServiceId;
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Networking::Sockets::StreamSocket;
    use windows::Storage::Streams::{DataReader, InputStreamOptions};

    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    info!(
        "fp-listen: starting Fast Pair listener for up to {}s (device filter: {:?})...",
        seconds, device_filter
    );

    let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?
        .get()
        .context("Failed to enumerate paired Bluetooth devices")?;

    let mut target_device = None;
    for dev in &devices {
        let name = dev.Name().unwrap_or_default().to_string();
        if let Some(filter) = device_filter {
            if name.to_lowercase().contains(&filter.to_lowercase()) {
                target_device = Some((name, dev.Id()?));
                break;
            }
        } else {
            // If no filter, take the first one that matches NeoBuds or first connected
            if name.to_lowercase().contains("neobuds") {
                target_device = Some((name, dev.Id()?));
                break;
            }
        }
    }

    // Fallback: if still None and no filter was given, pick the first device
    if target_device.is_none()
        && device_filter.is_none()
        && let Some(dev) = (&devices).into_iter().next()
    {
        let name = dev.Name().unwrap_or_default().to_string();
        target_device = Some((name, dev.Id()?));
    }

    let (device_name, device_id) = match target_device {
        Some(t) => t,
        None => {
            bail!(
                "No paired Bluetooth device found matching filter '{:?}'.",
                device_filter
            );
        }
    };

    info!(
        "fp-listen: target device '{}' (ID: {})",
        device_name, device_id
    );

    let bt_device = BluetoothDevice::FromIdAsync(&device_id)?
        .get()
        .with_context(|| format!("Failed to open BluetoothDevice for '{}'", device_name))?;

    let service_id = RfcommServiceId::FromUuid(FAST_PAIR_SERVICE_UUID)
        .context("Failed to create RfcommServiceId for Fast Pair UUID")?;

    info!(
        "fp-listen: querying RFCOMM services for Fast Pair UUID {{{:?}}}...",
        FAST_PAIR_SERVICE_UUID
    );
    let services_result = bt_device
        .GetRfcommServicesForIdAsync(&service_id)?
        .get()
        .context("Failed to query RFCOMM services on BluetoothDevice")?;

    let services = services_result.Services()?;
    if services.Size()? == 0 {
        bail!(
            "Device '{}' does not expose the Fast Pair RFCOMM service ({:?}).",
            device_name,
            FAST_PAIR_SERVICE_UUID
        );
    }

    let service = services.GetAt(0)?;
    let host_name = service.ConnectionHostName()?;
    let service_name = service.ConnectionServiceName()?;

    info!(
        "fp-listen: connecting StreamSocket to host '{:?}' / service '{}'...",
        host_name,
        service_name.to_string()
    );

    let socket = StreamSocket::new().context("Failed to create StreamSocket")?;
    socket
        .ConnectAsync(&host_name, &service_name)?
        .get()
        .context("Failed to connect StreamSocket to Fast Pair RFCOMM service")?;

    info!("fp-listen: CONNECTED! (strictly read-only, never writing)");
    let input_stream = socket.InputStream().context("Failed to get InputStream")?;
    let reader =
        DataReader::CreateDataReader(&input_stream).context("Failed to create DataReader")?;
    reader
        .SetInputStreamOptions(InputStreamOptions::Partial)
        .context("Failed to set InputStreamOptions")?;

    let mut parser = FrameParser::new();
    let start_time = Instant::now();
    let timeout = Duration::from_secs(seconds);

    println!(
        "\n>>> Listening on Fast Pair Message Stream for {}s... <<<\n",
        seconds
    );

    while start_time.elapsed() < timeout {
        // Read up to 256 bytes per chunk
        let load_op = reader.LoadAsync(256)?;
        let bytes_loaded = match load_op.get() {
            Ok(count) => count as usize,
            Err(e) => {
                warn!("fp-listen: LoadAsync returned error: {e}");
                break;
            }
        };

        if bytes_loaded == 0 {
            // With InputStreamOptions::Partial, 0 bytes means the remote closed the stream.
            warn!(
                "fp-listen: stream closed by the device after {:.2}s",
                start_time.elapsed().as_secs_f32()
            );
            break;
        }

        let mut chunk = vec![0u8; bytes_loaded];
        reader.ReadBytes(&mut chunk)?;

        let elapsed = start_time.elapsed().as_secs_f32();
        info!(
            "[+{:.2}s][FP RX] ({} bytes): {}",
            elapsed,
            chunk.len(),
            to_hex(&chunk)
        );

        let messages = parser.push(&chunk);
        for msg in messages {
            match msg {
                Message::ModelId(id) => {
                    info!(
                        "[+{:.2}s][FP FRAME] ModelId: {:02X} {:02X} {:02X}",
                        elapsed, id[0], id[1], id[2]
                    );
                    println!(
                        "🎉 [+{:.2}s] Model ID: {:02X}{:02X}{:02X}",
                        elapsed, id[0], id[1], id[2]
                    );
                }
                Message::Battery { left, right, case } => {
                    info!(
                        "[+{:.2}s][FP FRAME] Battery: Left={:?}(chg={}), Right={:?}(chg={}), Case={:?}(chg={})",
                        elapsed,
                        left.percent,
                        left.charging,
                        right.percent,
                        right.charging,
                        case.percent,
                        case.charging
                    );
                    println!(
                        "🔋 [+{:.2}s] Battery Update: L: {:?} (chg: {}) | R: {:?} (chg: {}) | Case: {:?} (chg: {})",
                        elapsed,
                        left.percent,
                        left.charging,
                        right.percent,
                        right.charging,
                        case.percent,
                        case.charging
                    );
                }
                Message::Other { group, code } => {
                    info!(
                        "[+{:.2}s][FP FRAME] Other message: group 0x{:02X}, code 0x{:02X}",
                        elapsed, group, code
                    );
                }
            }
        }
    }

    info!("fp-listen: finished {}s listen session.", seconds);
    println!("\n>>> Fast Pair listen session ended. <<<\n");
    Ok(())
}

#[cfg(not(windows))]
pub fn run_fp_listen(_device_filter: Option<&str>, _seconds: u64) -> Result<()> {
    println!("fp-listen is only supported on Windows.");
    Ok(())
}
