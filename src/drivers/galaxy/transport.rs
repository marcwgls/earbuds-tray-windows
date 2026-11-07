use anyhow::Result;
use std::sync::mpsc::Receiver;
use windows::core::GUID;

use crate::drivers::rfcomm::RfcommConnection;

/// Listen-only transport for Samsung Galaxy Buds.
///
/// Safety: This struct strictly has NO send method and never touches
/// the socket's output stream. One known Galaxy command (0xD4) dumps link
/// keys, so nothing may ever be written here.
pub struct GalaxyTransport {
    conn: RfcommConnection,
}

impl GalaxyTransport {
    pub fn connect(device_name_filter: &str, service_uuid: GUID) -> Result<Self> {
        let conn = RfcommConnection::connect(device_name_filter, service_uuid)?;
        Ok(Self { conn })
    }

    pub fn rx(&self) -> &Receiver<Vec<u8>> {
        self.conn.rx()
    }
}
