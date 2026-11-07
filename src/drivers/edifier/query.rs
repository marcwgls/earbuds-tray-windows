// Derived from mEDIFIER (MIT, wh201906) and WearEars (MIT, hdzw1155).
// See THIRD_PARTY_NOTICES.md for details.

use super::packet::{Frame, encode_v1, encode_v2};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    pub charging: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryF2 {
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    pub bud_in_case: bool,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    V1,
    V2Xor,
    V2NoXor,
}

/// Strictly bounded query commands.
/// No public API allows sending arbitrary bytes or dangerous commands (e.g. 0x07 Factory Reset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    BatteryD0,
    BatteryF2,
    AncState,
}

impl Query {
    pub fn cmd(self) -> u8 {
        match self {
            Query::BatteryD0 => 0xD0,
            Query::BatteryF2 => 0xF2,
            Query::AncState => 0xCC,
        }
    }

    /// Safely encodes this query into the requested protocol format.
    pub fn encode(self, fmt: Format) -> Vec<u8> {
        match fmt {
            Format::V1 => encode_v1(self.cmd(), &[]),
            Format::V2Xor => encode_v2(self.cmd(), &[], true),
            Format::V2NoXor => encode_v2(self.cmd(), &[], false),
        }
    }
}

/// Checks whether an incoming decoded frame matches the command of the query sent.
pub fn is_answer(sent: Query, frame: &Frame) -> bool {
    sent.cmd() == frame.cmd
}

/// Parses classic Edifier single battery payload (CMD 0xD0).
/// Format: `[value]` where value & 0x7F is percentage, and value & 0x80 indicates charging.
/// 0xFF indicates unknown.
pub fn parse_battery_d0(payload: &[u8]) -> Option<Battery> {
    if payload.is_empty() {
        return None;
    }
    let b = payload[0];
    if b == 0xFF {
        return None;
    }

    let val = b & 0x7F;
    let charging = (b & 0x80) != 0;

    Some(Battery {
        left: Some(val),
        right: None,
        case: None,
        charging: Some(charging),
    })
}

/// Parses Edifier TWS battery payload (CMD 0xF2).
/// Format: `[peer, L, R, ...]`
/// 0xFF or values > 100 indicate unknown / not present.
/// Remaining bytes are preserved in `raw`.
pub fn parse_battery_f2(payload: &[u8]) -> Option<BatteryF2> {
    if payload.len() < 3 {
        return None;
    }

    let left_raw = payload[1];
    let right_raw = payload[2];

    let left = if left_raw == 0xFF || left_raw > 100 {
        None
    } else {
        Some(left_raw)
    };
    let right = if right_raw == 0xFF || right_raw > 100 {
        None
    } else {
        Some(right_raw)
    };

    let case = if payload.len() > 3 && payload[3] != 0 && payload[3] <= 100 {
        Some(payload[3])
    } else {
        None
    };

    let bud_in_case = payload.len() > 4 && payload[4] == 0x02;

    Some(BatteryF2 {
        left,
        right,
        case,
        bud_in_case,
        raw: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::edifier::packet::Version;

    #[test]
    fn test_is_answer() {
        let f2_frame = Frame {
            version: Version::V2,
            cmd: 0xF2,
            payload: vec![],
        };
        let d0_frame = Frame {
            version: Version::V1,
            cmd: 0xD0,
            payload: vec![],
        };

        // Query::BatteryD0 with an F2 frame must return false
        assert!(!is_answer(Query::BatteryD0, &f2_frame));
        // Query::BatteryF2 with an F2 frame must return true
        assert!(is_answer(Query::BatteryF2, &f2_frame));
        // Query::BatteryD0 with a D0 frame must return true
        assert!(is_answer(Query::BatteryD0, &d0_frame));
        // Query::BatteryF2 with a D0 frame must return false
        assert!(!is_answer(Query::BatteryF2, &d0_frame));
    }

    #[test]
    fn test_query_encode_v1_d0() {
        let bytes = Query::BatteryD0.encode(Format::V1);
        assert_eq!(bytes, vec![0xAA, 0x01, 0xD0, 0x21, 0x94]);
    }

    #[test]
    fn test_query_encode_v2_f2() {
        let bytes = Query::BatteryF2.encode(Format::V2Xor);
        assert_eq!(bytes, vec![0xAA, 0xEC, 0xF2, 0x00, 0x00, 0x88]);
    }

    #[test]
    fn test_parse_battery_d0() {
        // Single battery 80% (0x50), not charging
        let payload = [0x50];
        let bat = parse_battery_d0(&payload).expect("should parse");
        assert_eq!(bat.left, Some(80));
        assert_eq!(bat.charging, Some(false));

        // Single battery 80% + charging bit 0x80 (0xD0)
        let payload_charging = [0xD0];
        let bat_c = parse_battery_d0(&payload_charging).expect("should parse");
        assert_eq!(bat_c.left, Some(80));
        assert_eq!(bat_c.charging, Some(true));

        // 0xFF means unknown
        let payload_ff = [0xFF];
        assert_eq!(parse_battery_d0(&payload_ff), None);
    }

    #[test]
    fn test_parse_battery_f2_no_bud_in_case() {
        // Run 2_6: both in ears
        let payload = [0x03, 0x4E, 0x51, 0x00, 0x03, 0x11];
        let bat = parse_battery_f2(&payload).expect("should parse");
        assert_eq!(bat.left, Some(78));
        assert_eq!(bat.right, Some(81));
        assert_eq!(bat.case, None);
        assert!(!bat.bud_in_case);
    }

    #[test]
    fn test_parse_battery_f2_bud_in_case_with_case_battery() {
        // Run 2_6: first bud into case -> case 52% (0x34), bud_in_case true (0x02)
        let payload = [0x03, 0x4E, 0x51, 0x34, 0x02, 0x11];
        let bat = parse_battery_f2(&payload).expect("should parse");
        assert_eq!(bat.left, Some(78));
        assert_eq!(bat.right, Some(81));
        assert_eq!(bat.case, Some(52));
        assert!(bat.bud_in_case);
    }

    #[test]
    fn test_parse_battery_f2_real_device_vector() {
        // Real payload from NeoBuds Pro 3: 03 63 64 00 03 11
        // left: 0x63 = 99%, right: 0x64 = 100%
        let payload = [0x03, 0x63, 0x64, 0x00, 0x03, 0x11];
        let bat = parse_battery_f2(&payload).expect("should parse");
        assert_eq!(bat.left, Some(99));
        assert_eq!(bat.right, Some(100));
        assert_eq!(bat.raw, vec![0x03, 0x63, 0x64, 0x00, 0x03, 0x11]);
    }

    #[test]
    fn test_parse_battery_f2_values_over_100_map_to_none() {
        // Un-XORed raw payload had 198 (0xC6) and 193 (0xC1), which are > 100
        let payload = [0x03, 198, 193, 0x00, 0x03, 0x11];
        let bat = parse_battery_f2(&payload).expect("should parse");
        assert_eq!(bat.left, None);
        assert_eq!(bat.right, None);
        assert_eq!(bat.raw, vec![0x03, 198, 193, 0x00, 0x03, 0x11]);
    }

    #[test]
    fn test_parse_battery_f2_ff_maps_to_none() {
        let payload = [0x01, 0xFF, 80, 0x00, 0x00];
        let bat = parse_battery_f2(&payload).expect("should parse");
        assert_eq!(bat.left, None);
        assert_eq!(bat.right, Some(80));
    }
}
