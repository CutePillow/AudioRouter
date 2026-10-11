use std::fmt;

pub const CONTROL_VERSION: u8 = 1;
pub const CAPABILITY_PCM: u32 = 0x01;
pub const CAPABILITY_OPUS: u32 = 0x04;
pub const CAPABILITY_PREFERENCE_SHIFT: u32 = 8;
pub const CAPABILITY_PREFERENCE_MASK: u32 = 0x03 << CAPABILITY_PREFERENCE_SHIFT;
pub const CAPABILITY_BITRATE_SHIFT: u32 = 10;
pub const CAPABILITY_BITRATE_MASK: u32 = 0x03ff << CAPABILITY_BITRATE_SHIFT;
pub const PREFERENCE_AUTO: u8 = 0;
pub const PREFERENCE_OPUS: u8 = 1;
pub const PREFERENCE_PCM: u8 = 2;

pub fn bluetooth_codec(local: u32, remote: u32) -> crate::Codec {
    let local_preference = capability_preference(local);
    let preference = if local_preference == PREFERENCE_AUTO {
        capability_preference(remote)
    } else {
        local_preference
    };
    let shared = local & remote;
    if preference == PREFERENCE_PCM {
        crate::Codec::Pcm16Le
    } else if shared & CAPABILITY_OPUS != 0 {
        crate::Codec::Opus
    } else {
        crate::Codec::Pcm16Le
    }
}
const MAX_DEVICE_NAME_LEN: usize = 128;

pub fn capability_preference(capabilities: u32) -> u8 {
    ((capabilities & CAPABILITY_PREFERENCE_MASK) >> CAPABILITY_PREFERENCE_SHIFT) as u8
}

pub fn capability_opus_bitrate_kbps(capabilities: u32) -> u32 {
    let bitrate = (capabilities & CAPABILITY_BITRATE_MASK) >> CAPABILITY_BITRATE_SHIFT;
    if bitrate == 0 {
        128
    } else {
        bitrate
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlMessage {
    Hello {
        device_id: u64,
        device_name: String,
        capabilities: u32,
    },
    HelloAck {
        device_id: u64,
        device_name: String,
        capabilities: u32,
    },
    SetSendEnabled(bool),
    SetReceiveEnabled(bool),
    SetSendVolume(u8),
    SetReceiveVolume(u8),
    Ping(u64),
    Pong(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlError {
    Empty,
    UnsupportedVersion(u8),
    UnknownType(u8),
    Truncated,
    InvalidDeviceName,
    InvalidVolume(u8),
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "control message is empty"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported control version: {version}")
            }
            Self::UnknownType(message_type) => {
                write!(formatter, "unknown control type: {message_type}")
            }
            Self::Truncated => write!(formatter, "truncated control message"),
            Self::InvalidDeviceName => write!(formatter, "invalid device name"),
            Self::InvalidVolume(volume) => write!(formatter, "invalid volume: {volume}"),
        }
    }
}

impl std::error::Error for ControlError {}

impl ControlMessage {
    pub fn encode(&self) -> Result<Vec<u8>, ControlError> {
        let mut bytes = vec![CONTROL_VERSION, self.message_type()];
        match self {
            Self::Hello {
                device_id,
                device_name,
                capabilities,
            }
            | Self::HelloAck {
                device_id,
                device_name,
                capabilities,
            } => {
                let name = device_name.as_bytes();
                if name.is_empty() || name.len() > MAX_DEVICE_NAME_LEN || device_name.contains('\0')
                {
                    return Err(ControlError::InvalidDeviceName);
                }
                bytes.extend_from_slice(&device_id.to_le_bytes());
                bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
                bytes.extend_from_slice(name);
                bytes.extend_from_slice(&capabilities.to_le_bytes());
            }
            Self::SetSendEnabled(enabled) | Self::SetReceiveEnabled(enabled) => {
                bytes.push(u8::from(*enabled));
            }
            Self::SetSendVolume(volume) | Self::SetReceiveVolume(volume) => {
                if *volume > 100 {
                    return Err(ControlError::InvalidVolume(*volume));
                }
                bytes.push(*volume);
            }
            Self::Ping(value) | Self::Pong(value) => bytes.extend_from_slice(&value.to_le_bytes()),
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() < 2 {
            return Err(ControlError::Empty);
        }
        if bytes[0] != CONTROL_VERSION {
            return Err(ControlError::UnsupportedVersion(bytes[0]));
        }
        let message_type = bytes[1];
        let body = &bytes[2..];
        match message_type {
            1 | 2 => {
                if body.len() < 12 {
                    return Err(ControlError::Truncated);
                }
                let device_id = read_u64(body, 0)?;
                let name_len = read_u16(body, 8)? as usize;
                let name_end = 10usize
                    .checked_add(name_len)
                    .ok_or(ControlError::Truncated)?;
                if name_len == 0 || name_len > MAX_DEVICE_NAME_LEN || body.len() < name_end + 4 {
                    return Err(ControlError::InvalidDeviceName);
                }
                let device_name = String::from_utf8(body[10..name_end].to_vec())
                    .map_err(|_| ControlError::InvalidDeviceName)?;
                let capabilities = read_u32(body, name_end)?;
                if body.len() != name_end + 4 {
                    return Err(ControlError::Truncated);
                }
                if message_type == 1 {
                    Ok(Self::Hello {
                        device_id,
                        device_name,
                        capabilities,
                    })
                } else {
                    Ok(Self::HelloAck {
                        device_id,
                        device_name,
                        capabilities,
                    })
                }
            }
            3 | 4 => {
                let enabled = body.first().copied().ok_or(ControlError::Truncated)?;
                if enabled > 1 || body.len() != 1 {
                    return Err(ControlError::Truncated);
                }
                if message_type == 3 {
                    Ok(Self::SetSendEnabled(enabled == 1))
                } else {
                    Ok(Self::SetReceiveEnabled(enabled == 1))
                }
            }
            5 | 6 => {
                let volume = body.first().copied().ok_or(ControlError::Truncated)?;
                if volume > 100 {
                    return Err(ControlError::InvalidVolume(volume));
                }
                if body.len() != 1 {
                    return Err(ControlError::Truncated);
                }
                if message_type == 5 {
                    Ok(Self::SetSendVolume(volume))
                } else {
                    Ok(Self::SetReceiveVolume(volume))
                }
            }
            7 | 8 => {
                if body.len() != 8 {
                    return Err(ControlError::Truncated);
                }
                let value = read_u64(body, 0)?;
                if message_type == 7 {
                    Ok(Self::Ping(value))
                } else {
                    Ok(Self::Pong(value))
                }
            }
            value => Err(ControlError::UnknownType(value)),
        }
    }

    fn message_type(&self) -> u8 {
        match self {
            Self::Hello { .. } => 1,
            Self::HelloAck { .. } => 2,
            Self::SetSendEnabled(_) => 3,
            Self::SetReceiveEnabled(_) => 4,
            Self::SetSendVolume(_) => 5,
            Self::SetReceiveVolume(_) => 6,
            Self::Ping(_) => 7,
            Self::Pong(_) => 8,
        }
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ControlError> {
    let end = offset.checked_add(2).ok_or(ControlError::Truncated)?;
    let value = bytes.get(offset..end).ok_or(ControlError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ControlError> {
    let end = offset.checked_add(4).ok_or(ControlError::Truncated)?;
    let value = bytes.get(offset..end).ok_or(ControlError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ControlError> {
    let end = offset.checked_add(8).ok_or(ControlError::Truncated)?;
    let value = bytes.get(offset..end).ok_or(ControlError::Truncated)?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_handshake() {
        let message = ControlMessage::Hello {
            device_id: 42,
            device_name: "Android 16".to_string(),
            capabilities: 0x11,
        };
        let encoded = message.encode().expect("encode");
        assert_eq!(ControlMessage::decode(&encoded).expect("decode"), message);
    }

    #[test]
    fn round_trip_preserves_volume_and_switches() {
        for message in [
            ControlMessage::SetSendEnabled(true),
            ControlMessage::SetReceiveEnabled(false),
            ControlMessage::SetSendVolume(37),
            ControlMessage::SetReceiveVolume(81),
            ControlMessage::Ping(123),
            ControlMessage::Pong(456),
        ] {
            let encoded = message.encode().expect("encode");
            assert_eq!(ControlMessage::decode(&encoded).expect("decode"), message);
        }
    }

    #[test]
    fn rejects_volume_above_percent_range() {
        assert_eq!(
            ControlMessage::SetSendVolume(101).encode(),
            Err(ControlError::InvalidVolume(101))
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut encoded = ControlMessage::SetReceiveVolume(50)
            .encode()
            .expect("encode");
        encoded.push(0);
        assert_eq!(
            ControlMessage::decode(&encoded),
            Err(ControlError::Truncated)
        );
    }

    #[test]
    fn decodes_codec_preference_and_bitrate_capabilities() {
        let capabilities = CAPABILITY_OPUS
            | (PREFERENCE_OPUS as u32) << CAPABILITY_PREFERENCE_SHIFT
            | 450 << CAPABILITY_BITRATE_SHIFT;

        assert_eq!(capability_preference(capabilities), PREFERENCE_OPUS);
        assert_eq!(capability_opus_bitrate_kbps(capabilities), 450);
    }
}
