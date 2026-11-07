use anyhow::Result;
use std::sync::mpsc::Receiver;
use windows::core::GUID;

use crate::drivers::rfcomm::RfcommConnection;

pub const FAST_PAIR_SERVICE_UUID: GUID = GUID::from_u128(0xDF21FE2C_2515_4FDB_8886_F12C4D67927C);

/// Listen-only transport for Google Fast Pair Message Stream.
///
/// Safety: This struct strictly has NO send method and never touches
/// the socket's output stream.
pub struct FastPairTransport {
    conn: RfcommConnection,
}

impl FastPairTransport {
    pub fn connect(device_name_filter: &str) -> Result<Self> {
        let conn = RfcommConnection::connect(device_name_filter, FAST_PAIR_SERVICE_UUID)?;
        Ok(Self { conn })
    }

    pub fn rx(&self) -> &Receiver<Vec<u8>> {
        self.conn.rx()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_pair_uuid_constant() {
        assert_eq!(
            FAST_PAIR_SERVICE_UUID,
            GUID::from_u128(0xDF21FE2C_2515_4FDB_8886_F12C4D67927C)
        );
    }
}
