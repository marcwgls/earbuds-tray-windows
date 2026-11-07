// Derived from win-bt-stereo-vs-handsfree (MIT + Attribution, Mark.Huang)
// and mEDIFIER (MIT, wh201906). See THIRD_PARTY_NOTICES.md for details.

use anyhow::Result;
use std::sync::mpsc::Receiver;
use windows::core::GUID;

use super::query::{Format, Query};
use crate::drivers::rfcomm::RfcommConnection;

/// Edifier Vendor Service UUID: EDF00000-EDFE-DFED-FEDF-EDFEDFEDFEDF
pub const EDIFIER_SERVICE_UUID: GUID = GUID::from_u128(0xEDF00000_EDFE_DFED_FEDF_EDFEDFEDFEDF);

pub struct EdifierTransport {
    conn: RfcommConnection,
}

impl EdifierTransport {
    /// Discovers a paired Bluetooth device matching `device_name_filter`,
    /// establishes an RFCOMM connection to the Edifier service UUID,
    /// and spawns a background thread streaming raw bytes to an mpsc channel.
    pub fn connect(device_name_filter: &str) -> Result<Self> {
        let conn = RfcommConnection::connect(device_name_filter, EDIFIER_SERVICE_UUID)?;
        Ok(Self { conn })
    }

    /// Encodes and safely sends a query across the established RFCOMM socket.
    pub fn send_query(&self, query: Query, fmt: Format) -> Result<()> {
        let bytes = query.encode(fmt);
        self.conn.send(&bytes)
    }

    /// Access the incoming byte stream receiver.
    pub fn rx(&self) -> &Receiver<Vec<u8>> {
        self.conn.rx()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edifier_uuid_constant() {
        assert_eq!(
            EDIFIER_SERVICE_UUID,
            GUID::from_u128(0xEDF00000_EDFE_DFED_FEDF_EDFEDFEDFEDF)
        );
    }
}
