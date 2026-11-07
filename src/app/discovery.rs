use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

use super::tiers::{Candidate, TierOverride, tier_for_with_override};

pub fn last_device_path(local_app_data: &Path) -> PathBuf {
    local_app_data.join("earbuds-tray").join("last-device.txt")
}

pub fn read_last_device_from(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn write_last_device_to(path: &Path, name: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, name.trim().as_bytes())
}

pub fn read_last_device() -> Option<String> {
    let app_data = std::env::var_os("LOCALAPPDATA")?;
    let path = last_device_path(Path::new(&app_data));
    read_last_device_from(&path)
}

pub fn write_last_device(name: &str) -> std::io::Result<()> {
    let app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA not found")
    })?;
    let path = last_device_path(Path::new(&app_data));
    write_last_device_to(&path, name)
}

#[cfg(windows)]
pub fn read_hfp_battery(container_id: &str) -> Option<u8> {
    use windows::Devices::Enumeration::Pnp::{PnpObject, PnpObjectType};
    use windows::Foundation::Collections::IIterable;
    use windows::Foundation::{IPropertyValue, PropertyType};
    use windows::core::{HSTRING, Interface};

    if container_id.is_empty() {
        return None;
    }

    let pnp_prop_name = HSTRING::from("{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2");
    let pnp_props: IIterable<HSTRING> = IIterable::try_from(vec![pnp_prop_name.clone()]).ok()?;
    let aqs = format!(r#"System.Devices.ContainerId:="{}""#, container_id);

    let pnp_op =
        PnpObject::FindAllAsyncAqsFilter(PnpObjectType::Device, &pnp_props, &HSTRING::from(aqs))
            .ok()?;
    let pnp_objs = pnp_op.get().ok()?;

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
                return Some(p);
            }
        }
    }

    None
}

#[cfg(not(windows))]
pub fn read_hfp_battery(_container_id: &str) -> Option<u8> {
    None
}

#[cfg(windows)]
pub fn is_device_connected(device_name: &str) -> bool {
    use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice};
    use windows::Devices::Enumeration::DeviceInformation;

    let selector = match BluetoothDevice::GetDeviceSelectorFromPairingState(true) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let devices = match DeviceInformation::FindAllAsyncAqsFilter(&selector) {
        Ok(op) => match op.get() {
            Ok(d) => d,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    for dev in &devices {
        let name = dev.Name().unwrap_or_default().to_string();
        if (name.eq_ignore_ascii_case(device_name)
            || name.to_lowercase().contains(&device_name.to_lowercase()))
            && let Ok(id) = dev.Id()
            && let Ok(bt_op) = BluetoothDevice::FromIdAsync(&id)
            && let Ok(bt_dev) = bt_op.get()
            && let Ok(status) = bt_dev.ConnectionStatus()
        {
            return status == BluetoothConnectionStatus::Connected;
        }
    }

    false
}

#[cfg(not(windows))]
pub fn is_device_connected(_device_name: &str) -> bool {
    false
}

#[cfg(windows)]
pub fn candidates(tier_override: Option<TierOverride>) -> Result<Vec<Candidate>> {
    use windows::Devices::Bluetooth::{
        BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice,
    };
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Foundation::Collections::IIterable;
    use windows::Foundation::{IPropertyValue, PropertyType};
    use windows::core::{HSTRING, Interface};

    let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    let props: IIterable<HSTRING> =
        IIterable::try_from(vec![HSTRING::from("System.Devices.Aep.ContainerId")])?;
    let paired_devices =
        DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties(&selector, &props)?
            .get()
            .context("Failed to enumerate paired devices")?;

    let mut result = Vec::new();

    for dev in &paired_devices {
        let name = dev.Name().unwrap_or_default().to_string();
        let aep_id = dev.Id().unwrap_or_default().to_string();

        let container_id = dev
            .Properties()
            .ok()
            .and_then(|p| {
                p.Lookup(&HSTRING::from("System.Devices.Aep.ContainerId"))
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
        let mut services = Vec::new();

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
                && let Ok(s_list) = services_res.Services()
            {
                for s in &s_list {
                    if let Ok(s_id) = s.ServiceId()
                        && let Ok(uuid) = s_id.Uuid()
                    {
                        services.push(uuid);
                    }
                }
            }
        }

        let has_hfp = read_hfp_battery(&container_id).is_some();
        if let Some(tier) = tier_for_with_override(&services, has_hfp, tier_override) {
            debug!(
                "Candidate found: '{}' (Connected: {}, Tier: {:?}, Container: {})",
                name, connected, tier, container_id
            );
            result.push(Candidate {
                name,
                container: container_id,
                connected,
                tier,
                services,
            });
        }
    }

    Ok(result)
}

#[cfg(not(windows))]
pub fn candidates(_tier_override: Option<TierOverride>) -> Result<Vec<Candidate>> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_last_device_path() {
        let base = Path::new(r"C:\Users\Marc\AppData\Local");
        let expected = PathBuf::from(r"C:\Users\Marc\AppData\Local\earbuds-tray\last-device.txt");
        assert_eq!(last_device_path(base), expected);
    }

    #[test]
    fn test_read_and_write_last_device() {
        let temp_dir = std::env::temp_dir().join("earbuds_tray_test_last_device");
        let _ = fs::remove_dir_all(&temp_dir);
        let path = temp_dir.join("last-device.txt");

        // Missing file returns None
        assert_eq!(read_last_device_from(&path), None);

        // Write and read back
        assert!(write_last_device_to(&path, "EDIFIER NeoBuds Pro 3\n").is_ok());
        assert_eq!(
            read_last_device_from(&path),
            Some("EDIFIER NeoBuds Pro 3".to_string())
        );

        // Blank content returns None
        assert!(write_last_device_to(&path, "   \n\t").is_ok());
        assert_eq!(read_last_device_from(&path), None);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
