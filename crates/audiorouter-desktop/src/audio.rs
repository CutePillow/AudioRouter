use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const OUTPUT_RATE: u32 = 48_000;
const OUTPUT_CHANNELS: usize = 2;
const OUTPUT_FRAME_BYTES: usize = OUTPUT_RATE as usize / 50 * OUTPUT_CHANNELS * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AudioEncoding {
    Pcm16,
    Pcm24,
    Pcm32,
    Float32,
}

impl AudioEncoding {
    fn bytes_per_sample(self) -> usize {
        match self {
            Self::Pcm16 => 2,
            Self::Pcm24 => 3,
            Self::Pcm32 | Self::Float32 => 4,
        }
    }
}

struct PcmConverter {
    input_rate: u32,
    input_channels: usize,
    encoding: AudioEncoding,
    pending: Vec<[f32; OUTPUT_CHANNELS]>,
    pending_start: u64,
    input_frames: u64,
    output_frames: u64,
}

impl PcmConverter {
    fn new(
        input_rate: u32,
        input_channels: usize,
        encoding: AudioEncoding,
    ) -> Result<Self, String> {
        if input_rate == 0 || !(1..=8).contains(&input_channels) {
            return Err("不支持的 Windows 混音格式".to_string());
        }
        Ok(Self {
            input_rate,
            input_channels,
            encoding,
            pending: Vec::new(),
            pending_start: 0,
            input_frames: 0,
            output_frames: 0,
        })
    }

    fn push(&mut self, input: &[u8], silent: bool) -> Result<Vec<u8>, String> {
        let bytes_per_sample = self.encoding.bytes_per_sample();
        let bytes_per_frame = bytes_per_sample * self.input_channels;
        if input.len() % bytes_per_frame != 0 {
            return Err("Windows 音频包未对齐完整采样帧".to_string());
        }
        for frame in input.chunks_exact(bytes_per_frame) {
            let left = if silent {
                0.0
            } else {
                decode_sample(&frame[..bytes_per_sample], self.encoding)
            };
            let right = if silent || self.input_channels == 1 {
                left
            } else {
                decode_sample(
                    &frame[bytes_per_sample..bytes_per_sample * 2],
                    self.encoding,
                )
            };
            self.pending.push([left, right]);
            self.input_frames += 1;
        }

        let mut output = Vec::new();
        loop {
            let source_numerator = self.output_frames * self.input_rate as u64;
            let source_index = source_numerator / OUTPUT_RATE as u64;
            let fraction = source_numerator % OUTPUT_RATE as u64;
            if source_index >= self.input_frames
                || (fraction != 0 && source_index + 1 >= self.input_frames)
            {
                break;
            }
            let local_index = (source_index - self.pending_start) as usize;
            let first = *self
                .pending
                .get(local_index)
                .ok_or_else(|| "重采样缓冲区状态无效".to_string())?;
            let sample = if fraction == 0 {
                first
            } else {
                let second = *self
                    .pending
                    .get(local_index + 1)
                    .ok_or_else(|| "重采样输入不足".to_string())?;
                let ratio = fraction as f32 / OUTPUT_RATE as f32;
                [
                    first[0] + (second[0] - first[0]) * ratio,
                    first[1] + (second[1] - first[1]) * ratio,
                ]
            };
            output.extend_from_slice(&float_to_pcm16(sample[0]).to_le_bytes());
            output.extend_from_slice(&float_to_pcm16(sample[1]).to_le_bytes());
            self.output_frames += 1;
        }

        let next_source_index = self.output_frames * self.input_rate as u64 / OUTPUT_RATE as u64;
        let trim_count = next_source_index.saturating_sub(self.pending_start) as usize;
        if trim_count > 0 {
            self.pending.drain(..trim_count.min(self.pending.len()));
            self.pending_start += trim_count as u64;
        }
        Ok(output)
    }
}

fn decode_sample(bytes: &[u8], encoding: AudioEncoding) -> f32 {
    match encoding {
        AudioEncoding::Pcm16 => i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0,
        AudioEncoding::Pcm24 => {
            let raw = (bytes[0] as i32) | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16);
            let signed = if raw & 0x80_0000 != 0 {
                raw | !0xff_ffff
            } else {
                raw
            };
            signed as f32 / 8_388_608.0
        }
        AudioEncoding::Pcm32 => {
            i32::from_le_bytes(bytes.try_into().expect("sample width")) as f32 / 2_147_483_648.0
        }
        AudioEncoding::Float32 => {
            let value = f32::from_le_bytes(bytes.try_into().expect("sample width"));
            if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }
    }
}

fn float_to_pcm16(value: f32) -> i16 {
    if value <= -1.0 {
        i16::MIN
    } else if value >= 1.0 {
        i16::MAX
    } else {
        (value.clamp(-1.0, 0.999_969_482_4) * 32_768.0)
            .round()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16
    }
}

pub(crate) fn apply_volume(pcm: &[u8], volume: u8) -> Vec<u8> {
    let mut output = pcm.to_vec();
    let gain = audiorouter_core::volume_to_gain(volume);
    if gain >= 0.999_999 {
        return output;
    }
    for sample in output.chunks_exact_mut(2) {
        let value = (i16::from_le_bytes([sample[0], sample[1]]) as f32 * gain)
            .round()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        sample.copy_from_slice(&value.to_le_bytes());
    }
    output
}

pub(crate) enum CaptureEvent {
    Frame(Vec<u8>),
    Error(String),
}

pub struct LoopbackCapture {
    stop: Arc<AtomicBool>,
    events: Receiver<CaptureEvent>,
    worker: Option<JoinHandle<()>>,
}

impl LoopbackCapture {
    pub fn start() -> Result<Self, String> {
        let (sender, events) = mpsc::sync_channel(3);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("AudioRouter-WASAPI-loopback".to_string())
            .spawn(move || {
                if let Err(error) = run_loopback(worker_stop, sender.clone()) {
                    let _ = sender.try_send(CaptureEvent::Error(error));
                }
            })
            .map_err(|error| format!("无法启动 Windows 音频捕获线程：{error}"))?;
        Ok(Self {
            stop,
            events,
            worker: Some(worker),
        })
    }

    pub(crate) fn poll(&self) -> Vec<CaptureEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn run_loopback(stop: Arc<AtomicBool>, sender: SyncSender<CaptureEvent>) -> Result<(), String> {
    windows_loopback::capture(stop, sender)
}

#[cfg(not(windows))]
fn run_loopback(_stop: Arc<AtomicBool>, _sender: SyncSender<CaptureEvent>) -> Result<(), String> {
    Err("WASAPI Loopback 仅支持 Windows".to_string())
}

#[cfg(windows)]
mod windows_loopback {
    use super::{AudioEncoding, CaptureEvent, PcmConverter, SyncSender, OUTPUT_FRAME_BYTES};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::TrySendError;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use windows::core::GUID;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    };

    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
    const SUBTYPE_PCM: GUID = GUID::from_u128(0x0000000100000010800000aa00389b71);
    const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x0000000300000010800000aa00389b71);

    struct MixFormat(*mut WAVEFORMATEX);

    impl Drop for MixFormat {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }

    pub(super) fn capture(
        stop: Arc<AtomicBool>,
        sender: SyncSender<CaptureEvent>,
    ) -> Result<(), String> {
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if initialized.is_err() {
            return Err(format!("初始化 Windows COM 失败：{initialized:?}"));
        }
        let result = capture_initialized(stop, sender);
        unsafe {
            CoUninitialize();
        }
        result
    }

    fn capture_initialized(
        stop: Arc<AtomicBool>,
        sender: SyncSender<CaptureEvent>,
    ) -> Result<(), String> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|error| format!("创建音频设备枚举器失败：{error}"))?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .map_err(|error| format!("找不到默认播放设备：{error}"))?;
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|error| format!("打开默认播放设备失败：{error}"))?;
        let format = MixFormat(
            unsafe { client.GetMixFormat() }
                .map_err(|error| format!("读取 Windows 混音格式失败：{error}"))?,
        );
        let wave = unsafe { std::ptr::read_unaligned(format.0) };
        let encoding = resolve_encoding(format.0, wave.wFormatTag, wave.wBitsPerSample)?;
        let mut converter =
            PcmConverter::new(wave.nSamplesPerSec, wave.nChannels as usize, encoding)?;
        let block_align = wave.nBlockAlign as usize;

        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                100_000,
                0,
                format.0,
                None,
            )
        }
        .map_err(|error| format!("启动 WASAPI Loopback 失败：{error}"))?;
        let capture: IAudioCaptureClient = unsafe { client.GetService() }
            .map_err(|error| format!("打开 WASAPI 捕获缓冲区失败：{error}"))?;
        unsafe { client.Start() }.map_err(|error| format!("启动 WASAPI 捕获失败：{error}"))?;

        let mut frame_buffer = Vec::with_capacity(OUTPUT_FRAME_BYTES * 2);
        while !stop.load(Ordering::Relaxed) {
            let next_packet = unsafe { capture.GetNextPacketSize() }
                .map_err(|error| format!("读取 WASAPI 缓冲区状态失败：{error}"))?;
            if next_packet == 0 {
                thread::sleep(Duration::from_millis(3));
                continue;
            }
            let mut frames = next_packet;
            let mut flags = 0u32;
            let mut data = std::ptr::null_mut();
            unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
                .map_err(|error| format!("读取 WASAPI 音频数据失败：{error}"))?;
            let byte_count = frames as usize * block_align;
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let input = if silent || data.is_null() {
                vec![0; byte_count]
            } else {
                unsafe { std::slice::from_raw_parts(data, byte_count) }.to_vec()
            };
            unsafe { capture.ReleaseBuffer(frames) }
                .map_err(|error| format!("释放 WASAPI 音频缓冲区失败：{error}"))?;

            let converted = converter.push(&input, silent)?;
            frame_buffer.extend_from_slice(&converted);
            while frame_buffer.len() >= OUTPUT_FRAME_BYTES {
                let frame: Vec<u8> = frame_buffer.drain(..OUTPUT_FRAME_BYTES).collect();
                match sender.try_send(CaptureEvent::Frame(frame)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => {
                        unsafe {
                            client.Stop().ok();
                        }
                        return Ok(());
                    }
                }
            }
        }
        unsafe { client.Stop() }.map_err(|error| format!("停止 WASAPI 捕获失败：{error}"))?;
        Ok(())
    }

    fn resolve_encoding(
        format: *mut WAVEFORMATEX,
        tag: u16,
        bits: u16,
    ) -> Result<AudioEncoding, String> {
        let subtype = if tag == WAVE_FORMAT_EXTENSIBLE {
            let extensible =
                unsafe { std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>()) };
            extensible.SubFormat
        } else {
            GUID::zeroed()
        };
        match (tag, bits, subtype) {
            (1, 16, _) | (WAVE_FORMAT_EXTENSIBLE, 16, SUBTYPE_PCM) => Ok(AudioEncoding::Pcm16),
            (1, 24, _) | (WAVE_FORMAT_EXTENSIBLE, 24, SUBTYPE_PCM) => Ok(AudioEncoding::Pcm24),
            (1, 32, _) | (WAVE_FORMAT_EXTENSIBLE, 32, SUBTYPE_PCM) => Ok(AudioEncoding::Pcm32),
            (WAVE_FORMAT_IEEE_FLOAT, 32, _) | (WAVE_FORMAT_EXTENSIBLE, 32, SUBTYPE_IEEE_FLOAT) => {
                Ok(AudioEncoding::Float32)
            }
            _ => Err(format!("不支持 Windows 混音格式：tag={tag}, bits={bits}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_volume, AudioEncoding, PcmConverter};

    #[test]
    fn pcm16_stereo_at_48khz_is_preserved() {
        let source = [0x00, 0x10, 0x00, 0xf0, 0xff, 0x7f, 0x00, 0x80];
        let mut converter = PcmConverter::new(48_000, 2, AudioEncoding::Pcm16).expect("converter");
        assert_eq!(converter.push(&source, false).expect("convert"), source);
    }

    #[test]
    fn mono_input_is_duplicated_to_stereo() {
        let source = [0x00, 0x20, 0x00, 0xe0];
        let mut converter = PcmConverter::new(48_000, 1, AudioEncoding::Pcm16).expect("converter");
        assert_eq!(
            converter.push(&source, false).expect("convert"),
            [0x00, 0x20, 0x00, 0x20, 0x00, 0xe0, 0x00, 0xe0]
        );
    }

    #[test]
    fn float_samples_convert_to_pcm16() {
        let source = [0x00, 0x00, 0x80, 0xbf, 0x00, 0x00, 0x00, 0x3f];
        let mut converter =
            PcmConverter::new(48_000, 1, AudioEncoding::Float32).expect("converter");
        assert_eq!(
            converter.push(&source, false).expect("convert"),
            [0x00, 0x80, 0x00, 0x80, 0x00, 0x40, 0x00, 0x40]
        );
    }

    #[test]
    fn resamples_across_input_boundaries() {
        let first_source = vec![0u8; 441 * 2];
        let final_source = vec![0u8; 2];
        let mut converter = PcmConverter::new(44_100, 1, AudioEncoding::Pcm16).expect("converter");
        assert_eq!(
            converter.push(&first_source, false).expect("convert").len(),
            479 * 4
        );
        assert_eq!(
            converter.push(&final_source, false).expect("convert").len(),
            8
        );
    }

    #[test]
    fn rejects_incomplete_input_frame() {
        let mut converter = PcmConverter::new(48_000, 2, AudioEncoding::Pcm16).expect("converter");
        assert!(converter.push(&[1, 2, 3], false).is_err());
    }

    #[test]
    fn volume_curve_scales_each_pcm_sample() {
        let source = [0x00, 0x40, 0x00, 0xc0];
        let scaled = apply_volume(&source, 50);
        assert!(i16::from_le_bytes([scaled[0], scaled[1]]).abs() > 7_000);
        assert!(i16::from_le_bytes([scaled[0], scaled[1]]).abs() < 9_000);
        assert!(i16::from_le_bytes([scaled[2], scaled[3]]).abs() > 7_000);
        assert!(i16::from_le_bytes([scaled[2], scaled[3]]).abs() < 9_000);
        assert_eq!(apply_volume(&source, 0), [0, 0, 0, 0]);
        assert_eq!(apply_volume(&source, 100), source);
    }
}
