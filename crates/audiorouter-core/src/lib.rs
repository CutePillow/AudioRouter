pub mod control;
pub mod packet;
pub mod volume;

pub use control::{
    bluetooth_codec, capability_opus_bitrate_kbps, capability_preference, ControlError,
    ControlMessage, CAPABILITY_BITRATE_MASK, CAPABILITY_BITRATE_SHIFT, CAPABILITY_OPUS,
    CAPABILITY_PCM, CAPABILITY_PREFERENCE_MASK, CAPABILITY_PREFERENCE_SHIFT, CONTROL_VERSION,
    PREFERENCE_AUTO, PREFERENCE_OPUS, PREFERENCE_PCM,
};
pub use packet::{
    AudioPacket, Codec, FrameDecoder, PacketError, PacketKind, PlaybackPacket, HEADER_LEN, MAGIC,
    MAX_PAYLOAD_LEN, PROTOCOL_VERSION,
};
pub use volume::{clamp_volume, volume_to_gain, MAX_VOLUME_PERCENT};
