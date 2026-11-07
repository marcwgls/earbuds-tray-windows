#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComponentBattery {
    pub percent: Option<u8>,
    pub charging: bool,
}

impl ComponentBattery {
    pub fn new(percent: Option<u8>, charging: bool) -> Self {
        Self { percent, charging }
    }
}

fn parse_component(b: u8) -> ComponentBattery {
    let charging = (b & 0x80) != 0;
    let raw_pct = b & 0x7F;
    let percent = if raw_pct <= 100 { Some(raw_pct) } else { None };
    ComponentBattery {
        percent,
        charging: if percent.is_some() { charging } else { false },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    ModelId([u8; 3]),
    Battery {
        left: ComponentBattery,
        right: ComponentBattery,
        case: ComponentBattery,
    },
    Other {
        group: u8,
        code: u8,
    },
}

#[derive(Debug, Default)]
pub struct FrameParser {
    buffer: Vec<u8>,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Message> {
        self.buffer.extend_from_slice(chunk);
        let mut messages = Vec::new();

        while self.buffer.len() >= 4 {
            let group = self.buffer[0];
            let code = self.buffer[1];
            let length = u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize;

            if self.buffer.len() < 4 + length {
                break;
            }

            let data = self.buffer[4..4 + length].to_vec();
            self.buffer.drain(..4 + length);

            // Fast Pair groups are 0x01..=0x05. Unknown groups are skipped.
            if !(1..=5).contains(&group) {
                continue;
            }

            match (group, code) {
                (0x03, 0x01) => {
                    if data.len() >= 3 {
                        messages.push(Message::ModelId([data[0], data[1], data[2]]));
                    } else {
                        messages.push(Message::Other { group, code });
                    }
                }
                (0x03, 0x03) => {
                    let left = data
                        .first()
                        .copied()
                        .map(parse_component)
                        .unwrap_or_default();
                    let right = data
                        .get(1)
                        .copied()
                        .map(parse_component)
                        .unwrap_or_default();
                    let case = data
                        .get(2)
                        .copied()
                        .map(parse_component)
                        .unwrap_or_default();
                    messages.push(Message::Battery { left, right, case });
                }
                _ => {
                    messages.push(Message::Other { group, code });
                }
            }
        }

        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_battery_basic_parsing() {
        let mut parser = FrameParser::new();
        let chunk = [0x03, 0x03, 0x00, 0x03, 0x57, 0x41, 0x7F];
        let msgs = parser.push(&chunk);
        assert_eq!(
            msgs,
            vec![Message::Battery {
                left: ComponentBattery::new(Some(87), false),
                right: ComponentBattery::new(Some(65), false),
                case: ComponentBattery::new(None, false),
            }]
        );
    }

    #[test]
    fn test_battery_charging_bit() {
        let mut parser = FrameParser::new();
        let chunk = [0x03, 0x03, 0x00, 0x03, 0xD7, 0x41, 0x7F];
        let msgs = parser.push(&chunk);
        assert_eq!(
            msgs,
            vec![Message::Battery {
                left: ComponentBattery::new(Some(87), true),
                right: ComponentBattery::new(Some(65), false),
                case: ComponentBattery::new(None, false),
            }]
        );
    }

    #[test]
    fn test_model_id_frame() {
        let mut parser = FrameParser::new();
        let chunk = [0x03, 0x01, 0x00, 0x03, 0xAA, 0xBB, 0xCC];
        let msgs = parser.push(&chunk);
        assert_eq!(msgs, vec![Message::ModelId([0xAA, 0xBB, 0xCC])]);
    }

    #[test]
    fn test_frame_split_across_two_pushes() {
        let mut parser = FrameParser::new();
        let push1 = [0x03, 0x03];
        let push2 = [0x00, 0x03, 0x57, 0x41, 0x7F];
        assert_eq!(parser.push(&push1), vec![]);
        assert_eq!(
            parser.push(&push2),
            vec![Message::Battery {
                left: ComponentBattery::new(Some(87), false),
                right: ComponentBattery::new(Some(65), false),
                case: ComponentBattery::new(None, false),
            }]
        );
    }

    #[test]
    fn test_two_frames_in_one_push() {
        let mut parser = FrameParser::new();
        let chunk = [
            0x03, 0x01, 0x00, 0x03, 0x01, 0x02, 0x03, // ModelId
            0x03, 0x03, 0x00, 0x03, 0x57, 0x41, 0x7F, // Battery
        ];
        let msgs = parser.push(&chunk);
        assert_eq!(
            msgs,
            vec![
                Message::ModelId([0x01, 0x02, 0x03]),
                Message::Battery {
                    left: ComponentBattery::new(Some(87), false),
                    right: ComponentBattery::new(Some(65), false),
                    case: ComponentBattery::new(None, false),
                }
            ]
        );
    }

    #[test]
    fn test_unknown_group_skipped() {
        let mut parser = FrameParser::new();
        let chunk = [0xFF, 0x01, 0x00, 0x01, 0x00];
        let msgs = parser.push(&chunk);
        assert_eq!(msgs, vec![]);
    }

    #[test]
    fn test_too_short_length_does_not_panic() {
        let mut parser = FrameParser::new();
        let chunk = [0x03, 0x01, 0x00, 0x01, 0xAA];
        let msgs = parser.push(&chunk);
        assert_eq!(msgs, vec![Message::Other { group: 3, code: 1 }]);

        let chunk2 = [0x03, 0x03, 0x00, 0x01, 0x57];
        let msgs2 = parser.push(&chunk2);
        assert_eq!(
            msgs2,
            vec![Message::Battery {
                left: ComponentBattery::new(Some(87), false),
                right: ComponentBattery::new(None, false),
                case: ComponentBattery::new(None, false),
            }]
        );
    }
}
