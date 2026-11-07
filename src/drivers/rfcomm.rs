use anyhow::{Context, Result, bail};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use tracing::{debug, info};
use windows::core::GUID;

#[cfg(windows)]
use windows::Devices::Bluetooth::BluetoothDevice;
#[cfg(windows)]
use windows::Devices::Bluetooth::Rfcomm::RfcommServiceId;
#[cfg(windows)]
use windows::Devices::Enumeration::DeviceInformation;
#[cfg(windows)]
use windows::Networking::Sockets::StreamSocket;
#[cfg(windows)]
use windows::Storage::Streams::{DataReader, DataWriter, InputStreamOptions};
#[cfg(windows)]
use windows::core::AgileReference;

pub struct RfcommConnection {
    #[cfg(windows)]
    socket: AgileReference<StreamSocket>,
    rx: Receiver<Vec<u8>>,
    #[cfg(windows)]
    _reader_handle: thread::JoinHandle<()>,
}

#[cfg(windows)]
impl RfcommConnection {
    /// Discovers a paired Bluetooth device matching `device_name_filter`,
    /// establishes an RFCOMM connection to `service_uuid`,
    /// and spawns a background thread streaming raw bytes to an mpsc channel.
    pub fn connect(device_name_filter: &str, service_uuid: GUID) -> Result<Self> {
        info!(
            "Searching for paired Bluetooth device matching '{}' for service {{{:?}}}...",
            device_name_filter, service_uuid
        );

        let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)
            .context("Failed to get Bluetooth device selector")?;
        let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?
            .get()
            .context("Failed to enumerate paired Bluetooth devices")?;

        let mut target_device_info = None;
        for dev in &devices {
            let name = dev.Name().unwrap_or_default().to_string();
            if name.eq_ignore_ascii_case(device_name_filter) {
                target_device_info = Some((name, dev.Id()?));
                break;
            }
        }
        if target_device_info.is_none() {
            for dev in &devices {
                let name = dev.Name().unwrap_or_default().to_string();
                if name
                    .to_lowercase()
                    .contains(&device_name_filter.to_lowercase())
                {
                    target_device_info = Some((name, dev.Id()?));
                    break;
                }
            }
        }

        let (device_name, device_id) = match target_device_info {
            Some((name, id)) => (name, id),
            None => {
                bail!(
                    "No paired Bluetooth device found matching '{}'. Make sure the headphones are paired and connected.",
                    device_name_filter
                );
            }
        };

        info!("Found device: '{}' (ID: {})", device_name, device_id);

        let bt_device = BluetoothDevice::FromIdAsync(&device_id)?
            .get()
            .with_context(|| format!("Failed to open BluetoothDevice for '{}'", device_name))?;

        let service_id = RfcommServiceId::FromUuid(service_uuid).with_context(|| {
            format!(
                "Failed to create RfcommServiceId for UUID {{{:?}}}",
                service_uuid
            )
        })?;

        info!(
            "Querying RFCOMM services for UUID {{{:?}}} on '{}'...",
            service_uuid, device_name
        );
        let services_result = bt_device
            .GetRfcommServicesForIdAsync(&service_id)?
            .get()
            .context("Failed to query RFCOMM services on BluetoothDevice")?;

        let services = services_result.Services()?;
        if services.Size()? == 0 {
            bail!(
                "Device '{}' does not expose the RFCOMM service ({:?}).",
                device_name,
                service_uuid
            );
        }

        let service = services.GetAt(0)?;
        let host_name = service.ConnectionHostName()?;
        let service_name = service.ConnectionServiceName()?;

        info!(
            "Connecting StreamSocket to host '{:?}' / service '{}'...",
            host_name,
            service_name.to_string()
        );

        let socket = StreamSocket::new().context("Failed to create StreamSocket")?;
        socket
            .ConnectAsync(&host_name, &service_name)?
            .get()
            .context("Failed to connect StreamSocket to RFCOMM service")?;

        info!("RFCOMM connection established successfully.");

        let (tx, rx) = channel::<Vec<u8>>();
        let input_stream = socket
            .InputStream()
            .context("Failed to get InputStream from socket")?;
        let agile_input = AgileReference::new(&input_stream)
            .context("Failed to create AgileReference for input stream")?;

        let reader_handle = thread::spawn(move || {
            let input_stream = match agile_input.resolve() {
                Ok(s) => s,
                Err(e) => {
                    debug!("Failed to resolve AgileReference: {e}");
                    return;
                }
            };
            let reader = match DataReader::CreateDataReader(&input_stream) {
                Ok(r) => r,
                Err(e) => {
                    debug!("Failed to create DataReader: {e}");
                    return;
                }
            };

            let _ = reader.SetInputStreamOptions(InputStreamOptions::Partial);

            loop {
                match reader.LoadAsync(512) {
                    Ok(async_op) => match async_op.get() {
                        Ok(loaded) => {
                            if loaded == 0 {
                                debug!("Socket connection closed by peer.");
                                break;
                            }
                            let mut buf = vec![0u8; loaded as usize];
                            if reader.ReadBytes(&mut buf).is_err() {
                                break;
                            }
                            if tx.send(buf).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            debug!("Reader LoadAsync error: {e}");
                            break;
                        }
                    },
                    Err(e) => {
                        debug!("Failed to start LoadAsync: {e}");
                        break;
                    }
                }
            }
        });

        let agile_socket =
            AgileReference::new(&socket).context("Failed to create AgileReference for socket")?;

        Ok(Self {
            socket: agile_socket,
            rx,
            _reader_handle: reader_handle,
        })
    }

    /// Access the incoming byte stream receiver.
    pub fn rx(&self) -> &Receiver<Vec<u8>> {
        &self.rx
    }

    /// Internal send method for vendor transports that write queries.
    pub(crate) fn send(&self, bytes: &[u8]) -> Result<()> {
        let socket = self
            .socket
            .resolve()
            .context("Failed to resolve socket AgileReference")?;
        let output_stream = socket
            .OutputStream()
            .context("Failed to get OutputStream")?;
        let writer =
            DataWriter::CreateDataWriter(&output_stream).context("Failed to create DataWriter")?;
        writer.WriteBytes(bytes)?;
        writer.StoreAsync()?.get()?;
        writer.DetachStream()?;
        Ok(())
    }
}

#[cfg(not(windows))]
impl RfcommConnection {
    pub fn connect(_device_name_filter: &str, _service_uuid: GUID) -> Result<Self> {
        bail!("RFCOMM is only supported on Windows");
    }

    pub fn rx(&self) -> &Receiver<Vec<u8>> {
        &self.rx
    }

    pub(crate) fn send(&self, _bytes: &[u8]) -> Result<()> {
        bail!("RFCOMM is only supported on Windows");
    }
}
