use super::status::{BatteryView, view_from_f2};
use crate::drivers::edifier::packet::Decoder;
use crate::drivers::edifier::query::parse_battery_f2;

pub struct Session {
    decoder: Decoder,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            decoder: Decoder::new(true),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Option<BatteryView> {
        let frames = self.decoder.push(bytes);
        let mut last_view = None;
        for frame in frames {
            if frame.cmd == 0xF2
                && let Some(battery_f2) = parse_battery_f2(&frame.payload)
                && let Some(view) = view_from_f2(&battery_f2)
            {
                last_view = Some(view);
            }
        }
        last_view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::edifier::packet::encode_v2;

    #[test]
    fn test_session_f2_response_chunk_gives_view() {
        let mut session = Session::new();
        // F2 payload: 03 [left] [right] [case] [bud_in_case] 11
        // 86% left, 89% right, 52% case, bud_in_case 0x02
        let f2_bytes = encode_v2(0xF2, &[0x03, 86, 89, 52, 0x02, 0x11], true);
        let view = session.push(&f2_bytes);
        assert_eq!(
            view,
            Some(BatteryView {
                left: Some(86),
                right: Some(89),
                case: Some(52),
            })
        );
    }

    #[test]
    fn test_session_cc_chunk_gives_none() {
        let mut session = Session::new();
        // CC frame (ANC push 20 06)
        let cc_bytes = encode_v2(0xCC, &[0x20, 0x06], true);
        let view = session.push(&cc_bytes);
        assert_eq!(view, None);

        // 0x50 frame (Now Playing)
        let cmd50_bytes = encode_v2(0x50, &[0x01, 0x02], true);
        let view2 = session.push(&cmd50_bytes);
        assert_eq!(view2, None);
    }

    #[test]
    fn test_session_chunk_with_cc_followed_by_f2_gives_f2_view() {
        let mut session = Session::new();
        let cc_bytes = encode_v2(0xCC, &[0x20, 0x06], true);
        let f2_bytes = encode_v2(0xF2, &[0x03, 75, 80, 0, 0x03, 0x11], true);

        let mut combined = cc_bytes;
        combined.extend_from_slice(&f2_bytes);

        let view = session.push(&combined);
        assert_eq!(
            view,
            Some(BatteryView {
                left: Some(75),
                right: Some(80),
                case: None, // 0 maps to None
            })
        );
    }

    #[test]
    fn test_session_f2_split_across_two_pushes() {
        let mut session = Session::new();
        let f2_bytes = encode_v2(0xF2, &[0x03, 60, 65, 40, 0x02, 0x11], true);
        assert!(f2_bytes.len() > 4);

        let half1 = &f2_bytes[..4];
        let half2 = &f2_bytes[4..];

        let view1 = session.push(half1);
        assert_eq!(view1, None);

        let view2 = session.push(half2);
        assert_eq!(
            view2,
            Some(BatteryView {
                left: Some(60),
                right: Some(65),
                case: Some(40),
            })
        );
    }
}
