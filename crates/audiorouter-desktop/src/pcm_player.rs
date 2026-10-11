use audiorouter_core::AudioPacket;

#[cfg(windows)]
mod windows_player {
    use super::AudioPacket;
    use std::collections::VecDeque;
    use std::mem::size_of;
    use std::thread;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Media::Audio::{
        waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader,
        waveOutWrite, CALLBACK_NULL, HWAVEOUT, WAVEFORMATEX, WAVEHDR, WAVE_FORMAT_PCM, WAVE_MAPPER,
        WHDR_DONE,
    };

    const MAX_PENDING_BUFFERS: usize = 3;

    struct PendingBuffer {
        _data: Vec<u8>,
        header: WAVEHDR,
    }

    pub struct Player {
        handle: HWAVEOUT,
        sample_rate: u32,
        channels: u8,
        volume: f32,
        pending: VecDeque<Box<PendingBuffer>>,
    }

    impl Player {
        pub fn new() -> Self {
            Self {
                handle: std::ptr::null_mut(),
                sample_rate: 0,
                channels: 0,
                volume: 1.0,
                pending: VecDeque::new(),
            }
        }

        pub fn set_volume(&mut self, volume: f32) {
            self.volume = volume.clamp(0.0, 1.0);
        }

        pub fn play(&mut self, packet: &AudioPacket) -> Result<(), String> {
            if packet.channels == 0 || packet.channels > 2 || packet.payload.is_empty() {
                return Ok(());
            }
            let frame_bytes = packet.channels as usize * 2;
            if packet.payload.len() % frame_bytes != 0 {
                return Err("Windows 音频帧长度与声道数不匹配".to_string());
            }
            if packet.sample_rate != self.sample_rate || packet.channels != self.channels {
                self.close();
                let format = WAVEFORMATEX {
                    wFormatTag: WAVE_FORMAT_PCM as u16,
                    nChannels: packet.channels as u16,
                    nSamplesPerSec: packet.sample_rate,
                    nAvgBytesPerSec: packet.sample_rate * packet.channels as u32 * 2,
                    nBlockAlign: packet.channels as u16 * 2,
                    wBitsPerSample: 16,
                    cbSize: 0,
                };
                let mut handle = std::ptr::null_mut();
                let result =
                    unsafe { waveOutOpen(&mut handle, WAVE_MAPPER, &format, 0, 0, CALLBACK_NULL) };
                if result != 0 || handle.is_null() {
                    return Err(format!("Windows 音频输出初始化失败：{result}"));
                }
                self.handle = handle;
                self.sample_rate = packet.sample_rate;
                self.channels = packet.channels;
            }
            self.reap_completed()?;
            self.wait_for_capacity()?;
            let mut data = packet.payload.clone();
            if self.volume < 0.999 {
                for bytes in data.chunks_exact_mut(2) {
                    let sample = i16::from_le_bytes([bytes[0], bytes[1]]);
                    let scaled = (sample as f32 * self.volume)
                        .round()
                        .clamp(i16::MIN as f32, i16::MAX as f32)
                        as i16;
                    bytes.copy_from_slice(&scaled.to_le_bytes());
                }
            }
            let mut pending = Box::new(PendingBuffer {
                _data: data,
                header: WAVEHDR {
                    lpData: std::ptr::null_mut(),
                    dwBufferLength: 0,
                    dwBytesRecorded: 0,
                    dwUser: 0,
                    dwFlags: 0,
                    dwLoops: 0,
                    lpNext: std::ptr::null_mut(),
                    reserved: 0,
                },
            });
            pending.header.lpData = pending._data.as_mut_ptr();
            pending.header.dwBufferLength = pending._data.len() as u32;
            let header_size = size_of::<WAVEHDR>() as u32;
            let prepared =
                unsafe { waveOutPrepareHeader(self.handle, &mut pending.header, header_size) };
            if prepared != 0 {
                return Err(format!("Windows 音频缓冲准备失败：{prepared}"));
            }
            let written = unsafe { waveOutWrite(self.handle, &mut pending.header, header_size) };
            if written != 0 {
                unsafe { waveOutUnprepareHeader(self.handle, &mut pending.header, header_size) };
                return Err(format!("Windows 音频写入失败：{written}"));
            }
            self.pending.push_back(pending);
            self.reap_completed()?;
            Ok(())
        }

        fn wait_for_capacity(&mut self) -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_millis(250);
            while self.pending.len() >= MAX_PENDING_BUFFERS {
                self.reap_completed()?;
                if self.pending.len() < MAX_PENDING_BUFFERS {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    unsafe {
                        waveOutReset(self.handle);
                    }
                    self.reap_completed()?;
                    return Err("Windows 音频输出缓冲跟不上输入".to_string());
                }
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        }

        fn reap_completed(&mut self) -> Result<(), String> {
            let header_size = size_of::<WAVEHDR>() as u32;
            loop {
                let done = self
                    .pending
                    .front()
                    .map(|pending| unsafe {
                        std::ptr::read_volatile(std::ptr::addr_of!(pending.header.dwFlags))
                            & WHDR_DONE
                            != 0
                    })
                    .unwrap_or(false);
                if !done {
                    return Ok(());
                }
                let Some(mut pending) = self.pending.pop_front() else {
                    return Ok(());
                };
                let result = unsafe {
                    waveOutUnprepareHeader(self.handle, &mut pending.header, header_size)
                };
                if result != 0 {
                    return Err(format!("Windows 音频缓冲释放失败：{result}"));
                }
            }
        }

        fn close(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    waveOutReset(self.handle);
                }
                let header_size = size_of::<WAVEHDR>() as u32;
                while let Some(mut pending) = self.pending.pop_front() {
                    unsafe {
                        waveOutUnprepareHeader(self.handle, &mut pending.header, header_size);
                    }
                }
                unsafe {
                    waveOutClose(self.handle);
                }
                self.handle = std::ptr::null_mut();
            }
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            self.close();
        }
    }
}

#[cfg(not(windows))]
mod windows_player {
    use super::AudioPacket;
    pub struct Player;
    impl Player {
        pub fn new() -> Self {
            Self
        }
        pub fn set_volume(&mut self, _volume: f32) {}
        pub fn play(&mut self, _packet: &AudioPacket) -> Result<(), String> {
            Ok(())
        }
    }
}

pub use windows_player::Player;
