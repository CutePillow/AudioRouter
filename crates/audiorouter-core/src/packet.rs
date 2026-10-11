use std::fmt;

pub const MAGIC: [u8; 4] = *b"ARAU";
pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 40;
pub const MAX_PAYLOAD_LEN: usize = 64 * 1024;

#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, PacketError> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        loop {
            if self.buffer.len() < HEADER_LEN {
                break;
            }
            if self.buffer[0..4] != MAGIC {
                return Err(PacketError::InvalidMagic);
            }
            if self.buffer[4] != PROTOCOL_VERSION {
                return Err(PacketError::UnsupportedVersion(self.buffer[4]));
            }
            let payload_len = read_u32(&self.buffer, 36) as usize;
            if payload_len > MAX_PAYLOAD_LEN {
                return Err(PacketError::PayloadTooLarge(payload_len));
            }
            let frame_len = HEADER_LEN
                .checked_add(payload_len)
                .ok_or(PacketError::PayloadTooLarge(payload_len))?;
            if self.buffer.len() < frame_len {
                break;
            }
            frames.push(self.buffer.drain(..frame_len).collect());
        }
        Ok(frames)
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PacketKind {
    Audio = 1,
    Hello = 2,
    HelloAck = 3,
    Ping = 4,
    Pong = 5,
}

impl TryFrom<u8> for PacketKind {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Audio),
            2 => Ok(Self::Hello),
            3 => Ok(Self::HelloAck),
            4 => Ok(Self::Ping),
            5 => Ok(Self::Pong),
            value => Err(PacketError::UnknownPacketKind(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Codec {
    Opus = 1,
    Pcm16Le = 2,
}

impl TryFrom<u8> for Codec {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Opus),
            2 => Ok(Self::Pcm16Le),
            value => Err(PacketError::UnknownCodec(value)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioPacket {
    pub session_id: u64,
    pub sequence: u32,
    pub timestamp_ms: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub codec: Codec,
    pub payload: Vec<u8>,
}

pub type PlaybackPacket = AudioPacket;

impl AudioPacket {
    pub fn new(
        session_id: u64,
        sequence: u32,
        timestamp_ms: u64,
        sample_rate: u32,
        channels: u8,
        payload: Vec<u8>,
    ) -> Result<Self, PacketError> {
        Self::new_with_codec(
            session_id,
            sequence,
            timestamp_ms,
            sample_rate,
            channels,
            Codec::Opus,
            payload,
        )
    }

    pub fn new_with_codec(
        session_id: u64,
        sequence: u32,
        timestamp_ms: u64,
        sample_rate: u32,
        channels: u8,
        codec: Codec,
        payload: Vec<u8>,
    ) -> Result<Self, PacketError> {
        let packet = Self {
            session_id,
            sequence,
            timestamp_ms,
            sample_rate,
            channels,
            codec,
            payload,
        };
        packet.validate()?;
        Ok(packet)
    }

    pub fn encode(&self) -> Result<Vec<u8>, PacketError> {
        self.validate()?;

        let payload_len = self.payload.len() as u32;
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&MAGIC);
        bytes.push(PROTOCOL_VERSION);
        bytes.push(PacketKind::Audio as u8);
        bytes.push(0);
        bytes.push(0);
        bytes.extend_from_slice(&self.session_id.to_le_bytes());
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        bytes.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.push(self.channels);
        bytes.push(self.codec as u8);
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&payload_len.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes.len() < HEADER_LEN {
            return Err(PacketError::Truncated {
                expected: HEADER_LEN,
                actual: bytes.len(),
            });
        }
        if bytes[0..4] != MAGIC {
            return Err(PacketError::InvalidMagic);
        }
        if bytes[4] != PROTOCOL_VERSION {
            return Err(PacketError::UnsupportedVersion(bytes[4]));
        }
        let packet_kind = PacketKind::try_from(bytes[5])?;
        if packet_kind != PacketKind::Audio {
            return Err(PacketError::UnexpectedPacketKind(packet_kind));
        }

        let session_id = read_u64(bytes, 8);
        let sequence = read_u32(bytes, 16);
        let timestamp_ms = read_u64(bytes, 20);
        let sample_rate = read_u32(bytes, 28);
        let channels = bytes[32];
        let codec = Codec::try_from(bytes[33])?;
        let payload_len = read_u32(bytes, 36) as usize;

        if payload_len > MAX_PAYLOAD_LEN {
            return Err(PacketError::PayloadTooLarge(payload_len));
        }

        let expected_len = HEADER_LEN
            .checked_add(payload_len)
            .ok_or(PacketError::PayloadTooLarge(payload_len))?;
        if bytes.len() != expected_len {
            return Err(PacketError::InvalidLength {
                expected: expected_len,
                actual: bytes.len(),
            });
        }

        let packet = Self {
            session_id,
            sequence,
            timestamp_ms,
            sample_rate,
            channels,
            codec,
            payload: bytes[HEADER_LEN..].to_vec(),
        };
        packet.validate()?;
        Ok(packet)
    }

    fn validate(&self) -> Result<(), PacketError> {
        if self.sample_rate == 0 {
            return Err(PacketError::InvalidSampleRate(self.sample_rate));
        }
        if self.channels == 0 {
            return Err(PacketError::InvalidChannels(self.channels));
        }
        if self.payload.len() > MAX_PAYLOAD_LEN {
            return Err(PacketError::PayloadTooLarge(self.payload.len()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PacketError {
    InvalidMagic,
    UnsupportedVersion(u8),
    UnknownPacketKind(u8),
    UnexpectedPacketKind(PacketKind),
    UnknownCodec(u8),
    Truncated { expected: usize, actual: usize },
    InvalidLength { expected: usize, actual: usize },
    PayloadTooLarge(usize),
    InvalidSampleRate(u32),
    InvalidChannels(u8),
}

impl fmt::Display for PacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(formatter, "invalid packet magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::UnknownPacketKind(kind) => write!(formatter, "unknown packet kind {kind}"),
            Self::UnexpectedPacketKind(kind) => {
                write!(formatter, "unexpected packet kind {kind:?}")
            }
            Self::UnknownCodec(codec) => write!(formatter, "unknown codec {codec}"),
            Self::Truncated { expected, actual } => {
                write!(
                    formatter,
                    "truncated packet: expected at least {expected} bytes, got {actual}"
                )
            }
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "invalid packet length: expected {expected} bytes, got {actual}"
                )
            }
            Self::PayloadTooLarge(length) => {
                write!(formatter, "payload is too large: {length} bytes")
            }
            Self::InvalidSampleRate(rate) => write!(formatter, "invalid sample rate {rate}"),
            Self::InvalidChannels(channels) => {
                write!(formatter, "invalid channel count {channels}")
            }
        }
    }
}

impl std::error::Error for PacketError {}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("packet field width"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("packet field width"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_packet() -> AudioPacket {
        AudioPacket::new(42, 7, 1234, 48_000, 2, vec![1, 2, 3, 4]).expect("valid packet")
    }

    #[test]
    fn round_trip_preserves_packet() {
        let packet = sample_packet();
        let encoded = packet.encode().expect("encode packet");
        let decoded = AudioPacket::decode(&encoded).expect("decode packet");
        assert_eq!(decoded, packet);
        assert_eq!(encoded.len(), HEADER_LEN + 4);
    }

    #[test]
    fn rejects_invalid_magic() {
        let mut encoded = sample_packet().encode().expect("encode packet");
        encoded[0] = b'X';
        assert_eq!(
            AudioPacket::decode(&encoded),
            Err(PacketError::InvalidMagic)
        );
    }

    #[test]
    fn rejects_truncated_header() {
        assert_eq!(
            AudioPacket::decode(&[0; HEADER_LEN - 1]),
            Err(PacketError::Truncated {
                expected: HEADER_LEN,
                actual: HEADER_LEN - 1,
            })
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut encoded = sample_packet().encode().expect("encode packet");
        encoded.push(0);
        assert_eq!(
            AudioPacket::decode(&encoded),
            Err(PacketError::InvalidLength {
                expected: HEADER_LEN + 4,
                actual: HEADER_LEN + 5,
            })
        );
    }

    #[test]
    fn rejects_oversized_payload() {
        let packet = AudioPacket {
            session_id: 1,
            sequence: 1,
            timestamp_ms: 1,
            sample_rate: 48_000,
            channels: 2,
            codec: Codec::Opus,
            payload: vec![0; MAX_PAYLOAD_LEN + 1],
        };
        assert_eq!(
            packet.encode(),
            Err(PacketError::PayloadTooLarge(MAX_PAYLOAD_LEN + 1))
        );
    }

    #[test]
    fn frame_decoder_handles_split_stream_frames() {
        let first = sample_packet().encode().expect("encode packet");
        let second = AudioPacket::new(42, 8, 1254, 48_000, 2, vec![5, 6])
            .expect("valid packet")
            .encode()
            .expect("encode packet");
        let mut decoder = FrameDecoder::default();
        let split_at = first.len() - 3;

        assert!(decoder
            .push(&first[..split_at])
            .expect("decode chunk")
            .is_empty());
        assert_eq!(decoder.buffered_len(), split_at);
        let frames = decoder
            .push(&[&first[split_at..], &second].concat())
            .expect("decode frames");

        assert_eq!(frames, vec![first, second]);
        assert_eq!(decoder.buffered_len(), 0);
    }
}
