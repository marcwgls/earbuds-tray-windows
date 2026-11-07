//! Listen-only decoder for the Samsung Galaxy Buds RFCOMM frames.
//!
//! Protocol notes: GalaxyBudsClient (GPL-3.0), see THIRD_PARTY_NOTICES.md.
//! This module only parses bytes. Nothing here can send.

/// Which frame layout the matched service UUID speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// Buds+ and newer: `FD`, header u16 LE, id, payload, CRC u16 LE, `DD`.
    New,
    /// Original Galaxy Buds: `FE`, type, size, id, payload, CRC u16 LE, `EE`.
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Status {
        left: Option<u8>,
        right: Option<u8>,
        case: Option<u8>,
    },
    Other(u8),
}

/// CRC16/XMODEM: poly 0x1021, init 0, no reflection.
pub fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

const MSG_STATUS: u8 = 0x60;
const MSG_EXTENDED_STATUS: u8 = 0x61;

enum Check {
    /// A complete, valid frame: total length, message id, payload.
    Valid(usize, u8, Vec<u8>),
    Invalid,
    Incomplete,
}

impl Framing {
    fn som(self) -> u8 {
        match self {
            Framing::New => 0xFD,
            Framing::Legacy => 0xFE,
        }
    }

    fn eom(self) -> u8 {
        match self {
            Framing::New => 0xDD,
            Framing::Legacy => 0xEE,
        }
    }

    /// Checks whether a frame starts at `buf[0]` (which must be the SOM).
    fn check(self, buf: &[u8]) -> Check {
        // `size` counts id + payload + CRC (2 bytes).
        let (size, header_len) = match self {
            Framing::New => {
                if buf.len() < 3 {
                    return Check::Incomplete;
                }
                let header = u16::from_le_bytes([buf[1], buf[2]]);
                if header & (1 << 13) != 0 {
                    // Fragmented messages are not supported.
                    return Check::Invalid;
                }
                ((header & 0x03FF) as usize, 3)
            }
            Framing::Legacy => {
                if buf.len() < 3 {
                    return Check::Incomplete;
                }
                (buf[2] as usize, 3)
            }
        };
        if size < 3 {
            return Check::Invalid;
        }
        let total = header_len + size + 1;
        if buf.len() < total {
            return Check::Incomplete;
        }
        if buf[total - 1] != self.eom() {
            return Check::Invalid;
        }
        let body = &buf[header_len..header_len + size - 2];
        let wire_crc = u16::from_le_bytes([buf[total - 3], buf[total - 2]]);
        if crc16_xmodem(body) != wire_crc {
            return Check::Invalid;
        }
        Check::Valid(total, body[0], body[1..].to_vec())
    }

    fn decode(self, id: u8, payload: &[u8]) -> Message {
        let get = |i: usize| payload.get(i).copied().and_then(battery);
        match (id, self) {
            (MSG_EXTENDED_STATUS, Framing::New) => Message::Status {
                left: get(2),
                right: get(3),
                case: get(7),
            },
            (MSG_EXTENDED_STATUS, Framing::Legacy) => Message::Status {
                left: get(2),
                right: get(3),
                case: None,
            },
            (MSG_STATUS, Framing::New) => Message::Status {
                left: get(1),
                right: get(2),
                case: get(6),
            },
            (MSG_STATUS, Framing::Legacy) => Message::Status {
                left: get(1),
                right: get(2),
                case: None,
            },
            _ => Message::Other(id),
        }
    }
}

/// 0 (not coupled or not reachable) and values over 100 count as unknown.
fn battery(raw: u8) -> Option<u8> {
    if (1..=100).contains(&raw) {
        Some(raw)
    } else {
        None
    }
}

#[derive(Debug)]
pub struct FrameParser {
    kind: Framing,
    buffer: Vec<u8>,
}

impl FrameParser {
    pub fn new(kind: Framing) -> Self {
        Self {
            kind,
            buffer: Vec::new(),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Message> {
        self.buffer.extend_from_slice(chunk);
        let mut messages = Vec::new();
        let som = self.kind.som();

        loop {
            match self.buffer.iter().position(|&b| b == som) {
                None => {
                    self.buffer.clear();
                    break;
                }
                Some(p) => {
                    self.buffer.drain(..p);
                }
            }

            match self.kind.check(&self.buffer) {
                Check::Valid(total, id, payload) => {
                    messages.push(self.kind.decode(id, &payload));
                    self.buffer.drain(..total);
                }
                // Never trust the claimed length: drop one byte and rescan.
                Check::Invalid => {
                    self.buffer.drain(..1);
                }
                Check::Incomplete => {
                    // A false SOM may claim more bytes than will ever come.
                    // If a complete valid frame starts later, skip to it.
                    let next = (1..self.buffer.len()).find(|&i| {
                        self.buffer[i] == som
                            && matches!(self.kind.check(&self.buffer[i..]), Check::Valid(..))
                    });
                    match next {
                        Some(i) => {
                            self.buffer.drain(..i);
                        }
                        None => break,
                    }
                }
            }
        }

        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDS2PRO: &[u8] = include_bytes!("fixtures/Buds2Pro_rev13.bin");
    const BUDSPLUS: &[u8] = include_bytes!("fixtures/BudsPlus_rev13.bin");
    const BUDS_LEGACY: &[u8] = include_bytes!("fixtures/Buds_rev3.bin");

    fn status(l: Option<u8>, r: Option<u8>, c: Option<u8>) -> Message {
        Message::Status {
            left: l,
            right: r,
            case: c,
        }
    }

    /// Test helper: builds a frame the way the buds do.
    fn encode(kind: Framing, id: u8, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![id];
        body.extend_from_slice(payload);
        let crc = crc16_xmodem(&body);
        let size = body.len() + 2;
        let mut out = Vec::new();
        match kind {
            Framing::New => {
                out.push(0xFD);
                out.extend_from_slice(&(size as u16).to_le_bytes());
            }
            Framing::Legacy => {
                out.push(0xFE);
                out.push(0x00);
                out.push(size as u8);
            }
        }
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc.to_le_bytes());
        out.push(if kind == Framing::New { 0xDD } else { 0xEE });
        out
    }

    #[test]
    fn crc_known_answer() {
        // CRC16/XMODEM check value.
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    #[test]
    fn fixture_buds2pro() {
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(
            p.push(BUDS2PRO),
            vec![status(Some(100), Some(100), Some(88))]
        );
    }

    #[test]
    fn fixture_budsplus() {
        // GalaxyBudsClient asserts L 87, R 84, case 101. 101 is over 100, so unknown.
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(p.push(BUDSPLUS), vec![status(Some(87), Some(84), None)]);
    }

    #[test]
    fn fixture_legacy_buds() {
        let mut p = FrameParser::new(Framing::Legacy);
        assert_eq!(p.push(BUDS_LEGACY), vec![status(Some(70), Some(95), None)]);
    }

    #[test]
    fn frame_split_over_two_chunks() {
        let mut p = FrameParser::new(Framing::New);
        let (a, b) = BUDS2PRO.split_at(20);
        assert_eq!(p.push(a), vec![]);
        assert_eq!(p.push(b), vec![status(Some(100), Some(100), Some(88))]);
    }

    #[test]
    fn frame_split_byte_by_byte() {
        let mut p = FrameParser::new(Framing::Legacy);
        let mut out = Vec::new();
        for b in BUDS_LEGACY {
            out.extend(p.push(&[*b]));
        }
        assert_eq!(out, vec![status(Some(70), Some(95), None)]);
    }

    #[test]
    fn two_frames_in_one_chunk() {
        let mut p = FrameParser::new(Framing::New);
        let mut chunk = BUDS2PRO.to_vec();
        chunk.extend_from_slice(BUDSPLUS);
        assert_eq!(
            p.push(&chunk),
            vec![
                status(Some(100), Some(100), Some(88)),
                status(Some(87), Some(84), None)
            ]
        );
    }

    #[test]
    fn flipped_crc_byte_is_dropped_and_next_frame_parses() {
        let mut p = FrameParser::new(Framing::New);
        let mut bad = BUDS2PRO.to_vec();
        let crc_pos = bad.len() - 3;
        bad[crc_pos] ^= 0x01;
        let mut chunk = bad;
        chunk.extend_from_slice(BUDSPLUS);
        assert_eq!(p.push(&chunk), vec![status(Some(87), Some(84), None)]);
    }

    #[test]
    fn wrong_crc_byte_order_is_not_accepted() {
        let mut p = FrameParser::new(Framing::New);
        let mut bad = BUDS2PRO.to_vec();
        let n = bad.len();
        bad.swap(n - 3, n - 2);
        assert_eq!(p.push(&bad), vec![]);
    }

    #[test]
    fn bad_eom_is_dropped() {
        let mut p = FrameParser::new(Framing::New);
        let mut bad = BUDS2PRO.to_vec();
        let n = bad.len();
        bad[n - 1] = 0x00;
        let mut chunk = bad;
        chunk.extend_from_slice(BUDSPLUS);
        assert_eq!(p.push(&chunk), vec![status(Some(87), Some(84), None)]);
    }

    #[test]
    fn fragment_bit_is_dropped() {
        let mut p = FrameParser::new(Framing::New);
        let mut frag = BUDS2PRO.to_vec();
        frag[2] |= 0x20; // bit 13 of the header
        let mut chunk = frag;
        chunk.extend_from_slice(BUDSPLUS);
        assert_eq!(p.push(&chunk), vec![status(Some(87), Some(84), None)]);
    }

    #[test]
    fn garbage_before_som() {
        let mut p = FrameParser::new(Framing::New);
        let mut chunk = vec![0x00, 0x11, 0x22, 0xDD, 0xFE];
        chunk.extend_from_slice(BUDS2PRO);
        assert_eq!(p.push(&chunk), vec![status(Some(100), Some(100), Some(88))]);
    }

    #[test]
    fn false_som_with_large_size_does_not_swallow_good_frame() {
        let mut p = FrameParser::new(Framing::New);
        // FD with size 500, far longer than what follows.
        let mut chunk = vec![0xFD, 0xF4, 0x01, 0x01, 0x02];
        chunk.extend_from_slice(BUDS2PRO);
        assert_eq!(p.push(&chunk), vec![status(Some(100), Some(100), Some(88))]);
    }

    #[test]
    fn false_som_legacy_with_large_size() {
        let mut p = FrameParser::new(Framing::Legacy);
        let mut chunk = vec![0xFE, 0x01, 0xC8, 0x01];
        chunk.extend_from_slice(BUDS_LEGACY);
        assert_eq!(p.push(&chunk), vec![status(Some(70), Some(95), None)]);
    }

    #[test]
    fn status_0x60_new() {
        // [0] rev, [1] L, [2] R, [3] coupled, [4] main, [5] placement, [6] case
        let frame = encode(Framing::New, 0x60, &[0x0D, 80, 60, 1, 0, 0x11, 40]);
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(p.push(&frame), vec![status(Some(80), Some(60), Some(40))]);
    }

    #[test]
    fn status_0x60_legacy() {
        // [0] ear type, [1] L, [2] R, no case
        let frame = encode(Framing::Legacy, 0x60, &[0x00, 55, 65]);
        let mut p = FrameParser::new(Framing::Legacy);
        assert_eq!(p.push(&frame), vec![status(Some(55), Some(65), None)]);
    }

    #[test]
    fn status_0x61_built_with_encoder() {
        let frame = encode(Framing::New, 0x61, &[0x0D, 0x00, 90, 85, 1, 0, 0x11, 70]);
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(p.push(&frame), vec![status(Some(90), Some(85), Some(70))]);
    }

    #[test]
    fn unknown_id_gives_other() {
        let frame = encode(Framing::New, 0x42, &[1, 2, 3]);
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(p.push(&frame), vec![Message::Other(0x42)]);
    }

    #[test]
    fn battery_zero_and_ff_are_unknown() {
        let frame = encode(Framing::New, 0x60, &[0x0D, 0, 0xFF, 1, 0, 0x11, 0]);
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(p.push(&frame), vec![status(None, None, None)]);
    }

    #[test]
    fn short_status_payload_does_not_panic() {
        let frame = encode(Framing::New, 0x61, &[0x0D]);
        let mut p = FrameParser::new(Framing::New);
        assert_eq!(p.push(&frame), vec![status(None, None, None)]);
    }

    #[test]
    fn legacy_parser_ignores_new_frames() {
        let mut p = FrameParser::new(Framing::Legacy);
        assert_eq!(p.push(BUDS2PRO), vec![]);
    }
}
