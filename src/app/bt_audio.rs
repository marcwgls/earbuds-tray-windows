// Based on approaches from ToothTray (m2jean) and bt-toggle (SQReder).

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::app::link::{AudioState, is_bt_audio_filter, pick_container};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtAudioDevice {
    pub container: String,
    pub name: String,
    pub state: AudioState,
    pub filters: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEvent {
    State(AudioState),
    CommandFailed { connect: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCmd {
    Connect,
    Disconnect,
    SetDevice(String),
    Quit,
}

pub fn spawn(
    device_filter: String,
    on_event: impl Fn(AudioEvent) + Send + 'static,
    commands: Receiver<AudioCmd>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        audio_loop(&device_filter, on_event, commands);
    })
}

fn audio_loop(device_filter: &str, on_event: impl Fn(AudioEvent), commands: Receiver<AudioCmd>) {
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    let mut current_filter = device_filter.to_string();
    let mut last_state = AudioState::Unknown;
    let mut poll_interval = Duration::from_secs(2);
    let mut last_poll = Instant::now() - poll_interval;
    let mut warned_error = false;

    loop {
        let time_to_next_poll = poll_interval.saturating_sub(last_poll.elapsed());
        match commands.recv_timeout(time_to_next_poll) {
            Ok(AudioCmd::Quit) => {
                info!("Audio thread received Quit command. Exiting.");
                break;
            }
            Ok(AudioCmd::SetDevice(new_filter)) => {
                info!(
                    "Audio thread filter updated: '{}' -> '{}'",
                    current_filter, new_filter
                );
                current_filter = new_filter;
                last_state = AudioState::Unknown;
                last_poll = Instant::now() - poll_interval;
            }
            Ok(cmd @ (AudioCmd::Connect | AudioCmd::Disconnect)) => {
                let connect = matches!(cmd, AudioCmd::Connect);
                info!(
                    "Audio thread processing {} command for '{}'...",
                    if connect { "Connect" } else { "Disconnect" },
                    current_filter
                );

                if current_filter.is_empty() {
                    warn!(
                        "Cannot {} audio: no device selected",
                        if connect { "connect" } else { "disconnect" }
                    );
                    on_event(AudioEvent::CommandFailed { connect });
                    continue;
                }

                let send_res = match find_device(&current_filter) {
                    Ok(Some(dev)) => send(&dev, connect),
                    Ok(None) => {
                        warn!(
                            "Cannot {} audio: device matching '{}' not found",
                            if connect { "connect" } else { "disconnect" },
                            current_filter
                        );
                        Err(windows::core::Error::new(
                            windows::core::HRESULT(-1),
                            "Device not found",
                        ))
                    }
                    Err(e) => {
                        warn!("find_device failed before send: {e}");
                        Err(e)
                    }
                };

                if let Err(e) = send_res {
                    warn!(
                        "Failed to send {} command: {e}",
                        if connect { "Connect" } else { "Disconnect" }
                    );
                    on_event(AudioEvent::CommandFailed { connect });
                }

                poll_interval = Duration::from_secs(1);
                last_poll = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {
                last_poll = Instant::now();
                poll_interval = Duration::from_secs(2);

                let current_state = if current_filter.is_empty() {
                    AudioState::NotFound
                } else {
                    match find_device(&current_filter) {
                        Ok(Some(dev)) => {
                            warned_error = false;
                            dev.state
                        }
                        Ok(None) => {
                            warned_error = false;
                            AudioState::NotFound
                        }
                        Err(e) => {
                            if !warned_error {
                                warn!("find_device failed in audio thread: {e}");
                                warned_error = true;
                            }
                            AudioState::Unknown
                        }
                    }
                };

                if current_state != last_state {
                    info!(
                        "Audio state changed: {:?} -> {:?}",
                        last_state, current_state
                    );
                    last_state = current_state;
                    on_event(AudioEvent::State(current_state));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                info!("Audio thread command channel disconnected. Exiting.");
                break;
            }
        }
    }
}

/// One audio endpoint as read from Windows: its container, name, state and
/// the Bluetooth audio filters behind it.
struct RawEndpoint {
    container_id: String,
    friendly_name: String,
    is_active: bool,
    filters: Vec<String>,
}

/// A device plus the names of all its endpoints, used for filter matching.
struct DeviceGroup {
    device: BtAudioDevice,
    names: Vec<String>,
}

/// Groups endpoints by container. Containers without any Bluetooth audio
/// filter (e.g. stale entries) are dropped, so they can never be picked.
fn group_endpoints(raw: Vec<RawEndpoint>) -> Vec<DeviceGroup> {
    let mut groups: Vec<DeviceGroup> = Vec::new();
    for ep in raw {
        let idx = match groups
            .iter()
            .position(|g| g.device.container == ep.container_id)
        {
            Some(i) => i,
            None => {
                groups.push(DeviceGroup {
                    device: BtAudioDevice {
                        container: ep.container_id.clone(),
                        name: String::new(),
                        state: AudioState::Disconnected,
                        filters: Vec::new(),
                    },
                    names: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[idx];
        if group.device.name.is_empty() {
            group.device.name = ep.friendly_name.clone();
        }
        if !ep.friendly_name.is_empty() {
            group.names.push(ep.friendly_name);
        }
        if ep.is_active {
            group.device.state = AudioState::Connected;
        }
        for f in ep.filters {
            if !group.device.filters.contains(&f) {
                group.device.filters.push(f);
            }
        }
    }
    groups.retain(|g| !g.device.filters.is_empty());
    for group in &mut groups {
        group.device.filters.sort();
    }
    groups.sort_by(|a, b| a.device.name.cmp(&b.device.name));
    groups
}

/// Picks the device whose endpoint names match `filter`.
fn select_device(groups: Vec<DeviceGroup>, filter: &str) -> Option<BtAudioDevice> {
    let pairs: Vec<(String, String)> = groups
        .iter()
        .flat_map(|g| {
            g.names
                .iter()
                .map(|n| (g.device.container.clone(), n.clone()))
        })
        .collect();
    let container = pick_container(&pairs, filter)?.to_string();
    let group = groups
        .into_iter()
        .find(|g| g.device.container == container)?;
    let filter_lower = filter.to_ascii_lowercase();
    let mut device = group.device;
    if let Some(name) = group
        .names
        .iter()
        .find(|n| n.to_ascii_lowercase().contains(&filter_lower))
    {
        device.name = name.clone();
    }
    Some(device)
}

#[cfg(windows)]
pub fn find_device(filter: &str) -> windows::core::Result<Option<BtAudioDevice>> {
    imp::find_device(filter)
}

#[cfg(windows)]
pub fn find_all_devices() -> windows::core::Result<Vec<BtAudioDevice>> {
    imp::find_all_devices()
}

#[cfg(windows)]
pub fn send(device: &BtAudioDevice, connect: bool) -> windows::core::Result<()> {
    imp::send(device, connect)
}

#[cfg(not(windows))]
pub fn find_device(_filter: &str) -> windows::core::Result<Option<BtAudioDevice>> {
    Ok(None)
}

#[cfg(not(windows))]
pub fn find_all_devices() -> windows::core::Result<Vec<BtAudioDevice>> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub fn send(_device: &BtAudioDevice, _connect: bool) -> windows::core::Result<()> {
    Ok(())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use tracing::{debug, info, warn};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Devices::Properties::DEVPKEY_Device_ContainerId;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Media::Audio::{
        DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATEMASK_ALL, IDeviceTopology, IMMDevice,
        IMMDeviceEnumerator, IPart, MMDeviceEnumerator, eAll,
    };
    use windows::Win32::Media::KernelStreaming::{
        IKsControl, IOCTL_KS_PROPERTY, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0,
        KSPROPERTY_ONESHOT_DISCONNECT, KSPROPERTY_ONESHOT_RECONNECT, KSPROPERTY_TYPE_GET,
        KSPROPSETID_BtAudio,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
    use windows::core::{HSTRING, Interface, PCWSTR};

    pub const PKEY_DEVICE_CONTAINER_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: DEVPKEY_Device_ContainerId.fmtid,
        pid: DEVPKEY_Device_ContainerId.pid,
    };

    /// The one-shot reconnect/disconnect request for the Bluetooth audio driver.
    fn bt_identifier(connect: bool) -> KSIDENTIFIER {
        let property = if connect {
            KSPROPERTY_ONESHOT_RECONNECT
        } else {
            KSPROPERTY_ONESHOT_DISCONNECT
        };
        KSIDENTIFIER {
            Anonymous: KSIDENTIFIER_0 {
                Anonymous: KSIDENTIFIER_0_0 {
                    Set: KSPROPSETID_BtAudio,
                    Id: property.0 as u32,
                    Flags: KSPROPERTY_TYPE_GET,
                },
            },
        }
    }

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

    fn inspect_endpoint(endpoint: &IMMDevice) -> Option<RawEndpoint> {
        let state = unsafe { endpoint.GetState().ok()? };
        let is_active = (state.0 & DEVICE_STATE_ACTIVE.0) != 0;

        let store = unsafe { endpoint.OpenPropertyStore(STGM_READ).ok()? };
        let friendly_name = unsafe {
            store
                .GetValue(&PKEY_Device_FriendlyName)
                .map(|v| v.to_string())
                .unwrap_or_default()
        };
        let container_id = unsafe {
            store
                .GetValue(&PKEY_DEVICE_CONTAINER_ID)
                .map(|v| v.to_string())
                .unwrap_or_default()
        };

        debug!(
            friendly_name = %friendly_name,
            container_id = %container_id,
            is_active,
            "Inspecting audio endpoint"
        );

        if container_id.is_empty() {
            debug!("Skipping endpoint with empty container_id: {friendly_name}");
            return None;
        }

        let mut filters = Vec::new();
        if let Ok(topo) = unsafe { endpoint.Activate::<IDeviceTopology>(CLSCTX_ALL, None) }
            && let Ok(conn_count) = unsafe { topo.GetConnectorCount() }
        {
            debug!(friendly_name = %friendly_name, conn_count, "DeviceTopology activated");
            for c in 0..conn_count {
                if let Ok(connector) = unsafe { topo.GetConnector(c) }
                    && let Ok(connected_to) = unsafe { connector.GetConnectedTo() }
                    && let Ok(part) = connected_to.cast::<IPart>()
                    && let Ok(hw_topo) = unsafe { part.GetTopologyObject() }
                    && let Ok(dev_id_pwstr) = unsafe { hw_topo.GetDeviceId() }
                {
                    if let Ok(id_str) = unsafe { dev_id_pwstr.to_string() } {
                        debug!(friendly_name = %friendly_name, filter_id = %id_str, "Found topology filter id");
                        if is_bt_audio_filter(&id_str) {
                            filters.push(id_str);
                        }
                    }
                    unsafe {
                        CoTaskMemFree(Some(dev_id_pwstr.as_ptr() as *const _));
                    }
                } else {
                    debug!(friendly_name = %friendly_name, connector_idx = c, "Connector walk failed or not connected");
                }
            }
        } else {
            debug!("Activate IDeviceTopology failed for {friendly_name}");
        }

        Some(RawEndpoint {
            container_id,
            friendly_name,
            is_active,
            filters,
        })
    }

    fn read_endpoints() -> windows::core::Result<Vec<RawEndpoint>> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let collection =
            unsafe { enumerator.EnumAudioEndpoints(eAll, DEVICE_STATE(DEVICE_STATEMASK_ALL))? };
        let count = unsafe { collection.GetCount()? };

        let mut raw_endpoints = Vec::new();
        for i in 0..count {
            if let Ok(endpoint) = unsafe { collection.Item(i) }
                && let Some(raw) = inspect_endpoint(&endpoint)
            {
                raw_endpoints.push(raw);
            }
        }
        Ok(raw_endpoints)
    }

    pub fn find_all_devices() -> windows::core::Result<Vec<BtAudioDevice>> {
        Ok(group_endpoints(read_endpoints()?)
            .into_iter()
            .map(|g| g.device)
            .collect())
    }

    pub fn find_device(filter: &str) -> windows::core::Result<Option<BtAudioDevice>> {
        Ok(select_device(group_endpoints(read_endpoints()?), filter))
    }

    fn send_ioctl_fallback(filter_id: &str, connect: bool) -> windows::core::Result<()> {
        let path_str = if let Some(idx) = filter_id.find(r"\\?\") {
            &filter_id[idx..]
        } else {
            filter_id
        };

        debug!("Opening device path for IOCTL fallback: {path_str}");
        let path_w: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR::from_raw(path_w.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                HANDLE::default(),
            )?
        };
        let _safe_handle = SafeHandle(handle);

        let prop = bt_identifier(connect);

        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                handle,
                IOCTL_KS_PROPERTY,
                Some(&prop as *const _ as *const _),
                std::mem::size_of::<KSIDENTIFIER>() as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )?;
        }

        Ok(())
    }

    /// Sends the request to every filter of the device. Windows only drops or
    /// restores the link once all of them (A2DP and Hands-Free) got it, so a
    /// failing filter is logged and the rest still get the request. Returns
    /// the first error.
    pub fn send(device: &BtAudioDevice, connect: bool) -> windows::core::Result<()> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };

        let mut first_err = None;
        for filter_id in &device.filters {
            // Safety guard: only send to {2}.\\?\bth filters
            if !is_bt_audio_filter(filter_id) {
                warn!("Skipping non-bt filter id: {filter_id}");
                continue;
            }
            if let Err(e) = send_one(&enumerator, filter_id, connect) {
                warn!(filter = %filter_id, connect, "KS property send failed: {e}");
                first_err.get_or_insert(e);
            }
        }

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn send_one(
        enumerator: &IMMDeviceEnumerator,
        filter_id: &str,
        connect: bool,
    ) -> windows::core::Result<()> {
        let dev = unsafe { enumerator.GetDevice(&HSTRING::from(filter_id))? };
        match unsafe { dev.Activate::<IKsControl>(CLSCTX_ALL, None) } {
            Ok(control) => {
                let prop = bt_identifier(connect);
                let mut returned = 0u32;
                unsafe {
                    control.KsProperty(
                        &prop,
                        std::mem::size_of::<KSIDENTIFIER>() as u32,
                        std::ptr::null_mut(),
                        0,
                        &mut returned,
                    )?;
                }
                info!(path = "IKsControl", filter = %filter_id, connect, "KS property sent");
                Ok(())
            }
            Err(e) if e.code() == windows::Win32::Foundation::E_NOINTERFACE => {
                warn!("IKsControl returned E_NOINTERFACE; trying DeviceIoControl for {filter_id}");
                send_ioctl_fallback(filter_id, connect)?;
                info!(path = "DeviceIoControl", filter = %filter_id, connect, "KS property sent");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(container: &str, name: &str, active: bool, filters: &[&str]) -> RawEndpoint {
        RawEndpoint {
            container_id: container.to_string(),
            friendly_name: name.to_string(),
            is_active: active,
            filters: filters.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[test]
    fn select_device_skips_container_without_bt_filters() {
        let raw = vec![
            ep("{stale}", "Kopfhörer (NeoBuds Pro 3)", false, &[]),
            ep(
                "{live}",
                "Kopfhörer (NeoBuds Pro 3)",
                true,
                &[r"{2}.\?\bthenum#a"],
            ),
            ep(
                "{live}",
                "Headset (NeoBuds Pro 3 Hands-Free)",
                false,
                &[r"{2}.\?\bthhfenum#b"],
            ),
        ];
        let device = select_device(group_endpoints(raw), "NeoBuds Pro 3").unwrap();
        assert_eq!(device.container, "{live}");
        assert_eq!(device.state, AudioState::Connected);
        assert_eq!(
            device.filters,
            vec![
                r"{2}.\?\bthenum#a".to_string(),
                r"{2}.\?\bthhfenum#b".to_string()
            ]
        );
    }

    #[test]
    fn group_endpoints_drops_containers_without_filters() {
        let raw = vec![
            ep("{realtek}", "Speakers (Realtek)", true, &[]),
            ep(
                "{buds}",
                "Kopfhörer (NeoBuds Pro 3)",
                false,
                &[r"{2}.\?\bthenum#a"],
            ),
        ];
        let groups = group_endpoints(raw);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].device.container, "{buds}");
        assert_eq!(groups[0].device.state, AudioState::Disconnected);
    }

    #[test]
    fn select_device_returns_none_without_match() {
        let raw = vec![ep(
            "{buds}",
            "Kopfhörer (NeoBuds Pro 3)",
            true,
            &[r"{2}.\?\bthenum#a"],
        )];
        assert!(select_device(group_endpoints(raw), "AirPods").is_none());
    }
}
