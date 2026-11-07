// Derived from mEDIFIER (MIT, wh201906) and WearEars (MIT, hdzw1155).
// See THIRD_PARTY_NOTICES.md for details.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: Version,
    pub cmd: u8,
    pub payload: Vec<u8>,
}

/// Encodes a V1 packet: `[AA, len(data)+1, cmd, data.., sum_hi, sum_lo]`
/// sum = 0x2019 + Summe aller vorherigen Bytes (u16, wrapping), Big Endian.
pub fn encode_v1(cmd: u8, data: &[u8]) -> Vec<u8> {
    let len = (data.len() + 1) as u8;
    let mut packet = Vec::with_capacity(5 + data.len());
    packet.push(0xAA);
    packet.push(len);
    packet.push(cmd);
    packet.extend_from_slice(data);

    let mut sum: u16 = 0x2019;
    for &b in &packet {
        sum = sum.wrapping_add(b as u16);
    }

    packet.push((sum >> 8) as u8);
    packet.push((sum & 0xFF) as u8);
    packet
}

/// Encodes a V2 packet: `[AA, EC, cmd, len_hi, len_lo, data^A5.., crc8]`
pub fn encode_v2(cmd: u8, data: &[u8], xor: bool) -> Vec<u8> {
    let len = data.len();
    let mut packet = Vec::with_capacity(6 + len);
    packet.push(0xAA);
    packet.push(0xEC);
    packet.push(cmd);
    packet.push((len >> 8) as u8);
    packet.push((len & 0xFF) as u8);

    if xor {
        for &b in data {
            packet.push(b ^ 0xA5);
        }
    } else {
        packet.extend_from_slice(data);
    }

    let crc8 = packet.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    packet.push(crc8);
    packet
}

pub struct Decoder {
    buffer: Vec<u8>,
    xor: bool,
}

impl Decoder {
    pub fn new(xor: bool) -> Self {
        Self {
            buffer: Vec::new(),
            xor,
        }
    }

    /// Pushes incoming raw bytes, buffers fragments, resynchronizes on corrupt bytes,
    /// and returns any successfully decoded and verified frames.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();

        while !self.buffer.is_empty() {
            // Need at least 2 bytes to determine potential frame type
            if self.buffer.len() < 2 {
                break;
            }

            // Check V2 format: [HEAD, 0xEC, CMD, LEN_H, LEN_L, PAYLOAD..., CRC8]
            if self.buffer[1] == 0xEC {
                if self.buffer.len() < 6 {
                    // Incomplete V2 header, wait for more bytes
                    break;
                }
                let cmd = self.buffer[2];
                let payload_len = ((self.buffer[3] as usize) << 8) | (self.buffer[4] as usize);
                let total_len = 5 + payload_len + 1; // 5 header bytes + payload + 1 crc8

                if payload_len > 1024 {
                    // Plausibility check failed, discard first byte to resync
                    self.buffer.remove(0);
                    continue;
                }

                if self.buffer.len() < total_len {
                    // Incomplete V2 payload, wait for more bytes
                    break;
                }

                // Verify CRC8: sum of all bytes up to CRC8
                let expected_crc = self.buffer[..total_len - 1]
                    .iter()
                    .fold(0u8, |acc, &b| acc.wrapping_add(b));
                let actual_crc = self.buffer[total_len - 1];

                if expected_crc == actual_crc {
                    let mut payload = self.buffer[5..5 + payload_len].to_vec();
                    if self.xor {
                        for b in &mut payload {
                            *b ^= 0xA5;
                        }
                    }
                    frames.push(Frame {
                        version: Version::V2,
                        cmd,
                        payload,
                    });
                    self.buffer.drain(..total_len);
                    continue;
                } else {
                    // CRC mismatch, resync by discarding one byte
                    self.buffer.remove(0);
                    continue;
                }
            }

            // Check V1 format: [HEAD, LEN, CMD, PAYLOAD..., CRC_HI, CRC_LO]
            // Valid V1 response headers are typically 0xBB, 0xCC, or 0xAA.
            let head = self.buffer[0];
            if head == 0xBB || head == 0xCC || head == 0xAA {
                let len = self.buffer[1] as usize; // len = 1 (CMD) + payload.len()
                if len == 0 || len > 250 {
                    // Invalid length for V1 frame, discard head byte
                    self.buffer.remove(0);
                    continue;
                }

                // Total length: 1 (head) + 1 (len) + len + 2 (crc16) = len + 4
                let total_len = len + 4;
                if self.buffer.len() < total_len {
                    // Incomplete V1 frame, wait for more bytes
                    break;
                }

                let cmd = self.buffer[2];
                let mut sum: u16 = 0x2019;
                for &b in &self.buffer[..total_len - 2] {
                    sum = sum.wrapping_add(b as u16);
                }

                let expected_hi = (sum >> 8) as u8;
                let expected_lo = (sum & 0xFF) as u8;
                let actual_hi = self.buffer[total_len - 2];
                let actual_lo = self.buffer[total_len - 1];

                if expected_hi == actual_hi && expected_lo == actual_lo {
                    let payload = self.buffer[3..total_len - 2].to_vec();
                    frames.push(Frame {
                        version: Version::V1,
                        cmd,
                        payload,
                    });
                    self.buffer.drain(..total_len);
                    continue;
                } else {
                    // CRC mismatch, resync
                    self.buffer.remove(0);
                    continue;
                }
            }

            // Neither V2 nor V1 header match, discard byte to resync
            self.buffer.remove(0);
        }

        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_v1_d0_vector() {
        let expected = vec![0xAA, 0x01, 0xD0, 0x21, 0x94];
        let actual = encode_v1(0xD0, &[]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_encode_v2_f2_vector() {
        let expected = vec![0xAA, 0xEC, 0xF2, 0x00, 0x00, 0x88];
        let actual = encode_v2(0xF2, &[], true);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_decoder_splits_across_pushes() {
        let mut decoder = Decoder::new(false);
        // Valid V1 frame: BB 01 D0 21 A5 (sum = 0x2019 + 0xBB + 0x01 + 0xD0 = 0x21A5)
        let frame_bytes = [0xBB, 0x01, 0xD0, 0x21, 0xA5];
        let half1 = &frame_bytes[..2];
        let half2 = &frame_bytes[2..];

        let res1 = decoder.push(half1);
        assert_eq!(res1, vec![]);

        let res2 = decoder.push(half2);
        assert_eq!(
            res2,
            vec![Frame {
                version: Version::V1,
                cmd: 0xD0,
                payload: vec![],
            }]
        );
    }

    #[test]
    fn test_decoder_rejects_corrupted_checksum() {
        let mut decoder = Decoder::new(false);
        // Corrupted checksum: last byte 0x00 instead of 0xA5
        let corrupted = vec![0xBB, 0x01, 0xD0, 0x21, 0x00];
        let res = decoder.push(&corrupted);
        assert_eq!(res, vec![]);
    }

    #[test]
    fn test_decoder_skips_garbage_resync() {
        let mut decoder = Decoder::new(false);
        // Preceding garbage byte 0xFF
        let mut bytes = vec![0xFF];
        bytes.extend_from_slice(&[0xBB, 0x01, 0xD0, 0x21, 0xA5]);

        let res = decoder.push(&bytes);
        assert_eq!(
            res,
            vec![Frame {
                version: Version::V1,
                cmd: 0xD0,
                payload: vec![],
            }]
        );
    }

    #[test]
    fn test_decoder_v2_frame() {
        let mut decoder = Decoder::new(false);
        // V2 frame: [BB, EC, F2, 00, 01, 5A, CRC8]
        // sum = 0xBB + 0xEC + 0xF2 + 0x00 + 0x01 + 0x5A = 187 + 236 + 242 + 0 + 1 + 90 = 756 = 0x02F4.
        // CRC8 = 0xF4
        let v2_bytes = vec![0xBB, 0xEC, 0xF2, 0x00, 0x01, 0x5A, 0xF4];
        let res = decoder.push(&v2_bytes);
        assert_eq!(
            res,
            vec![Frame {
                version: Version::V2,
                cmd: 0xF2,
                payload: vec![0x5A],
            }]
        );
    }

    #[test]
    fn test_decoder_v2_xor_real_device_frame() {
        let mut decoder = Decoder::new(true);
        // Real device frame: BB EC F2 00 06 A6 C6 C1 A5 A6 B4 CB
        // Payload after XOR 0xA5: 03 63 64 00 03 11 (left 99, right 100)
        let frame_bytes = [
            0xBB, 0xEC, 0xF2, 0x00, 0x06, 0xA6, 0xC6, 0xC1, 0xA5, 0xA6, 0xB4, 0xCB,
        ];
        let res = decoder.push(&frame_bytes);
        assert_eq!(
            res,
            vec![Frame {
                version: Version::V2,
                cmd: 0xF2,
                payload: vec![0x03, 0x63, 0x64, 0x00, 0x03, 0x11],
            }]
        );
    }

    #[test]
    fn test_decoder_v2_xor_passive_frame() {
        let mut decoder = Decoder::new(true);
        // Real passive frame: BB EC F2 00 06 A6 C7 C1 A5 A6 B4 CC
        // Payload after XOR 0xA5: 03 62 64 00 03 11 (left 98, right 100)
        let frame_bytes = [
            0xBB, 0xEC, 0xF2, 0x00, 0x06, 0xA6, 0xC7, 0xC1, 0xA5, 0xA6, 0xB4, 0xCC,
        ];
        let res = decoder.push(&frame_bytes);
        assert_eq!(
            res,
            vec![Frame {
                version: Version::V2,
                cmd: 0xF2,
                payload: vec![0x03, 0x62, 0x64, 0x00, 0x03, 0x11],
            }]
        );
    }
}
