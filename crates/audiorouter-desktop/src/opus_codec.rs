use opus::{Application, Bitrate, Channels, Decoder, Encoder};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES_PER_CHANNEL: usize = 960;

pub struct OpusFrameEncoder {
    channels: usize,
    bitrate_kbps: u32,
    encoder: Encoder,
    pending: Vec<i16>,
}

impl OpusFrameEncoder {
    pub fn new(channels: u8) -> Result<Self, String> {
        Self::new_with_bitrate(channels, if channels == 1 { 32 } else { 80 })
    }

    pub fn new_with_bitrate(channels: u8, bitrate_kbps: u32) -> Result<Self, String> {
        let channels = channel_count(channels)?;
        let bitrate_kbps = bitrate_kbps.clamp(6, 450);
        let opus_channels = opus_channels(channels);
        let mut encoder = Encoder::new(SAMPLE_RATE, opus_channels, Application::Audio)
            .map_err(|error| format!("创建 Opus 编码器失败：{error}"))?;
        encoder
            .set_bitrate(Bitrate::Bits((bitrate_kbps * 1_000) as i32))
            .map_err(|error| format!("设置 Opus 码率失败：{error}"))?;
        Ok(Self {
            channels,
            bitrate_kbps,
            encoder,
            pending: Vec::new(),
        })
    }

    pub fn bitrate_kbps(&self) -> u32 {
        self.bitrate_kbps
    }

    pub fn push_pcm16le(&mut self, pcm: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        if pcm.len() % 2 != 0 {
            return Err("PCM16LE 数据长度必须为偶数".to_string());
        }
        self.pending.extend(
            pcm.chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]])),
        );
        let frame_len = FRAME_SAMPLES_PER_CHANNEL * self.channels;
        let mut packets = Vec::new();
        while self.pending.len() >= frame_len {
            let frame: Vec<i16> = self.pending.drain(..frame_len).collect();
            let mut encoded = vec![0u8; 4000];
            let length = self
                .encoder
                .encode(&frame, &mut encoded)
                .map_err(|error| format!("Opus 编码失败：{error}"))?;
            encoded.truncate(length);
            packets.push(encoded);
        }
        Ok(packets)
    }
}

pub struct OpusFrameDecoder {
    decoder: Decoder,
    channels: usize,
}

impl OpusFrameDecoder {
    pub fn new(channels: u8) -> Result<Self, String> {
        let channels = channel_count(channels)?;
        let decoder = Decoder::new(SAMPLE_RATE, opus_channels(channels))
            .map_err(|error| format!("创建 Opus 解码器失败：{error}"))?;
        Ok(Self { decoder, channels })
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<u8>, String> {
        if packet.is_empty() {
            return Err("Opus 音频包不能为空".to_string());
        }
        let mut decoded = vec![0i16; FRAME_SAMPLES_PER_CHANNEL * self.channels * 2];
        let samples_per_channel = self
            .decoder
            .decode(packet, &mut decoded, false)
            .map_err(|error| format!("Opus 解码失败：{error}"))?;
        decoded.truncate(samples_per_channel * self.channels);
        let mut pcm = Vec::with_capacity(decoded.len() * 2);
        for sample in decoded {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(pcm)
    }
}

fn channel_count(channels: u8) -> Result<usize, String> {
    match channels {
        1 | 2 => Ok(channels as usize),
        _ => Err(format!("Opus 只支持单声道或双声道，收到 {channels}")),
    }
}

fn opus_channels(channels: usize) -> Channels {
    if channels == 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    }
}

#[cfg(test)]
mod tests {
    use super::{OpusFrameDecoder, OpusFrameEncoder};

    #[test]
    fn buffers_pcm_until_a_complete_twenty_millisecond_frame_is_available() {
        let mut encoder = OpusFrameEncoder::new(1).expect("create Opus encoder");
        let pcm = vec![0; 1_920];

        assert!(encoder
            .push_pcm16le(&pcm[..960])
            .expect("buffer partial frame")
            .is_empty());
        let packets = encoder.push_pcm16le(&pcm[960..]).expect("complete frame");

        assert_eq!(packets.len(), 1);
        assert!(packets[0].len() < pcm.len());
    }

    #[test]
    fn opus_round_trip_preserves_twenty_millisecond_stereo_duration_and_channels() {
        let mut encoder = OpusFrameEncoder::new(2).expect("create Opus encoder");
        let mut pcm = Vec::with_capacity(3_840);
        for sample_index in 0..960 {
            let left_sample = ((sample_index as f32 * 0.08).sin() * 20_000.0) as i16;
            let right_sample = ((sample_index as f32 * 0.16).sin() * 20_000.0) as i16;
            pcm.extend_from_slice(&left_sample.to_le_bytes());
            pcm.extend_from_slice(&right_sample.to_le_bytes());
        }
        let packet = encoder
            .push_pcm16le(&pcm)
            .expect("encode frame")
            .into_iter()
            .next()
            .expect("one Opus packet");
        let mut decoder = OpusFrameDecoder::new(2).expect("create Opus decoder");

        let decoded = decoder.decode(&packet).expect("decode Opus packet");

        assert_eq!(decoded.len(), pcm.len());
        let distinct_samples = decoded
            .chunks_exact(4)
            .filter(|frame| {
                let left_sample = i16::from_le_bytes([frame[0], frame[1]]) as i32;
                let right_sample = i16::from_le_bytes([frame[2], frame[3]]) as i32;
                (left_sample - right_sample).abs() > 100
            })
            .count();
        assert!(distinct_samples > 480, "左右声道不应被合并或复制");
    }

    #[test]
    fn rejects_pcm_with_a_partial_sample() {
        let mut encoder = OpusFrameEncoder::new(1).expect("create Opus encoder");

        assert!(encoder.push_pcm16le(&[0]).is_err());
    }

    #[test]
    fn accepts_the_high_quality_450_kbps_profile() {
        let encoder =
            OpusFrameEncoder::new_with_bitrate(2, 450).expect("create high bitrate Opus encoder");

        assert_eq!(encoder.bitrate_kbps(), 450);
    }
}
