#![windows_subsystem = "windows"]

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use audiorouter_core::{
    volume_to_gain, AudioPacket, Codec, ControlMessage, PREFERENCE_AUTO, PREFERENCE_OPUS,
    PREFERENCE_PCM,
};
use eframe::egui;
use serde_json::{json, Value};

mod audio;
mod bluetooth;
mod lan;
mod opus_codec;
mod pcm_player;
mod retry;
mod settings;
#[cfg(windows)]
mod tray;
mod webview;

#[cfg(windows)]
mod single_instance {
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    pub struct Guard(*mut core::ffi::c_void);

    impl Guard {
        pub fn acquire() -> Option<Self> {
            let name: Vec<u16> = "Local\\AudioRouter.Desktop.SingleInstance\0"
                .encode_utf16()
                .collect();
            let handle = unsafe { CreateMutexW(null_mut(), 1, name.as_ptr()) };
            if handle.is_null() {
                return None;
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                return None;
            }
            Some(Self(handle))
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

use audio::{CaptureEvent, LoopbackCapture};
use bluetooth::{BluetoothServer, ServerEvent as BluetoothEvent};
use lan::{LanServer, ServerEvent};
use pcm_player::Player;
use settings::{AppTheme, DesktopSettings, DeviceSettings};

struct StartupRegistrationWorker {
    request_tx: Sender<(u64, bool, Option<bool>)>,
    result_rx: Receiver<(u64, bool, Option<bool>, Result<(), String>)>,
    next_request_id: u64,
}

impl StartupRegistrationWorker {
    fn new(initial_enabled: bool) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<(u64, bool, Option<bool>)>();
        let (result_tx, result_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("startup-registration".to_string())
            .spawn(move || {
                if initial_enabled {
                    let result = DesktopSettings::apply_startup_registration(true);
                    let _ = result_tx.send((0, true, None, result));
                }
                while let Ok((request_id, enabled, previous_enabled)) = request_rx.recv() {
                    let result = DesktopSettings::apply_startup_registration(enabled);
                    if result_tx
                        .send((request_id, enabled, previous_enabled, result))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("创建开机自启动后台工作线程失败");
        Self {
            request_tx,
            result_rx,
            next_request_id: 0,
        }
    }

    fn request(&mut self, enabled: bool, previous_enabled: bool) -> Result<u64, String> {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;
        self.request_tx
            .send((request_id, enabled, Some(previous_enabled)))
            .map_err(|error| format!("无法更新开机自启动设置：{error}"))?;
        Ok(request_id)
    }
}

fn configure_fonts(context: &egui::Context) {
    let font_paths = [
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    let Some(font_bytes) = font_paths.iter().find_map(|path| std::fs::read(path).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "audiorouter_cjk".to_string(),
        egui::FontData::from_owned(font_bytes).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(font_family) = fonts.families.get_mut(&family) {
            font_family.insert(0, "audiorouter_cjk".to_string());
        }
    }
    context.set_fonts(fonts);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportKind {
    Lan,
    Bluetooth,
}

impl TransportKind {
    fn title(self) -> &'static str {
        match self {
            Self::Lan => "局域网连接",
            Self::Bluetooth => "蓝牙连接",
        }
    }

    fn accent(self) -> egui::Color32 {
        match self {
            Self::Lan => egui::Color32::from_rgb(224, 154, 22),
            Self::Bluetooth => egui::Color32::from_rgb(34, 180, 112),
        }
    }
}

struct DeviceCard {
    key: String,
    transport: TransportKind,
    name: String,
    detail: String,
    connected: bool,
    connected_at: Option<Instant>,
    send_enabled: bool,
    receive_enabled: bool,
    send_volume: u8,
    receive_volume: u8,
    lan_address: Option<SocketAddr>,
    bluetooth_address: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsCategory {
    Appearance,
    Personalization,
    Transport,
    Buffer,
}

struct AudioRouterApp {
    send_playback: bool,
    capture_requested: bool,
    lan_server: Option<LanServer>,
    bluetooth_server: Option<BluetoothServer>,
    devices: HashMap<String, DeviceCard>,
    bluetooth_scan_at: Instant,
    last_action: String,
    transport_error: Option<String>,
    playback_tx: PlaybackQueue,
    playback_error_rx: Receiver<String>,
    playback_played_count: Arc<AtomicU64>,
    playback_error_count: Arc<AtomicU64>,
    loopback_capture: Option<LoopbackCapture>,
    send_session_id: u64,
    send_sequence: u32,
    lan_opus_encoders: HashMap<SocketAddr, opus_codec::OpusFrameEncoder>,
    lan_opus_decoders: HashMap<(SocketAddr, u64, u8), opus_codec::OpusFrameDecoder>,
    bluetooth_opus_encoders: HashMap<usize, opus_codec::OpusFrameEncoder>,
    bluetooth_opus_decoders: HashMap<(u64, u8), opus_codec::OpusFrameDecoder>,
    bluetooth_received_packets: u64,
    bluetooth_decoded_packets: u64,
    bluetooth_played_packets: u64,
    bluetooth_playback_errors: u64,
    playback_dropped_packets: u64,
    settings: DesktopSettings,
    startup_registration: StartupRegistrationWorker,
    startup_registration_request_id: u64,
    settings_open: bool,
    settings_category: Option<SettingsCategory>,
    show_settings_lock: bool,
    lan_refresh_until: Option<Instant>,
    bluetooth_refresh_until: Option<Instant>,
    auto_connect_tasks: HashMap<String, retry::RetryTask>,
    suppressed_auto_targets: HashSet<String>,
}

const PLAYBACK_QUEUE_CAPACITY: usize = 12;
const PLAYBACK_STARTUP_BUFFER_FRAMES: usize = 2;

#[derive(Clone)]
struct PlaybackQueue {
    inner: Arc<(Mutex<VecDeque<(AudioPacket, f32)>>, Condvar)>,
}

impl PlaybackQueue {
    fn new() -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(VecDeque::with_capacity(PLAYBACK_QUEUE_CAPACITY)),
                Condvar::new(),
            )),
        }
    }

    fn push_latest(&self, item: (AudioPacket, f32), capacity: usize) -> Result<bool, ()> {
        let (items, ready) = &*self.inner;
        let mut items = items.lock().map_err(|_| ())?;
        let dropped_oldest = items.len() >= capacity;
        if dropped_oldest {
            items.pop_front();
        }
        items.push_back(item);
        ready.notify_one();
        Ok(dropped_oldest)
    }

    fn pop(&self) -> Option<(AudioPacket, f32)> {
        let (items, ready) = &*self.inner;
        let mut items = items.lock().ok()?;
        loop {
            if let Some(item) = items.pop_front() {
                return Some(item);
            }
            items = ready.wait(items).ok()?;
        }
    }
}

fn start_playback_worker() -> (
    PlaybackQueue,
    Receiver<String>,
    Arc<AtomicU64>,
    Arc<AtomicU64>,
) {
    let playback_tx = PlaybackQueue::new();
    let playback_rx = playback_tx.clone();
    let (error_tx, error_rx) = mpsc::sync_channel(8);
    let played_count = Arc::new(AtomicU64::new(0));
    let error_count = Arc::new(AtomicU64::new(0));
    let worker_played_count = Arc::clone(&played_count);
    let worker_error_count = Arc::clone(&error_count);

    let _ = std::thread::Builder::new()
        .name("AudioRouter-playback".to_string())
        .spawn(move || {
            let mut player = Player::new();
            let mut prebuffer = VecDeque::with_capacity(PLAYBACK_STARTUP_BUFFER_FRAMES);
            while prebuffer.len() < PLAYBACK_STARTUP_BUFFER_FRAMES {
                let Some(item) = playback_rx.pop() else {
                    return;
                };
                prebuffer.push_back(item);
            }
            while let Some((packet, volume)) = prebuffer.pop_front() {
                player.set_volume(volume);
                match player.play(&packet) {
                    Ok(()) => {
                        worker_played_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        worker_error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = error_tx.try_send(error);
                    }
                }
            }
            while let Some((packet, volume)) = playback_rx.pop() {
                player.set_volume(volume);
                match player.play(&packet) {
                    Ok(()) => {
                        worker_played_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        worker_error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = error_tx.try_send(error);
                    }
                }
            }
        });

    (playback_tx, error_rx, played_count, error_count)
}

impl Default for AudioRouterApp {
    fn default() -> Self {
        let settings = DesktopSettings::load();
        let restore_lan = settings.restore_connection_state && settings.lan_enabled;
        let restore_bluetooth = settings.restore_connection_state && settings.bluetooth_enabled;
        let startup_registration = StartupRegistrationWorker::new(settings.launch_on_startup);
        let (playback_tx, playback_error_rx, playback_played_count, playback_error_count) =
            start_playback_worker();
        let mut app = Self {
            send_playback: false,
            capture_requested: false,
            lan_server: None,
            bluetooth_server: None,
            devices: HashMap::new(),
            bluetooth_scan_at: Instant::now() - Duration::from_secs(60),
            last_action: "请选择要启用的连接方式".to_string(),
            transport_error: None,
            playback_tx,
            playback_error_rx,
            playback_played_count,
            playback_error_count,
            loopback_capture: None,
            send_session_id: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            send_sequence: 0,
            lan_opus_encoders: HashMap::new(),
            lan_opus_decoders: HashMap::new(),
            bluetooth_opus_encoders: HashMap::new(),
            bluetooth_opus_decoders: HashMap::new(),
            bluetooth_received_packets: 0,
            bluetooth_decoded_packets: 0,
            bluetooth_played_packets: 0,
            bluetooth_playback_errors: 0,
            playback_dropped_packets: 0,
            settings,
            startup_registration,
            startup_registration_request_id: 0,
            settings_open: false,
            settings_category: None,
            show_settings_lock: false,
            lan_refresh_until: None,
            bluetooth_refresh_until: None,
            auto_connect_tasks: HashMap::new(),
            suppressed_auto_targets: HashSet::new(),
        };
        if restore_lan {
            app.toggle_lan(true);
        }
        if restore_bluetooth {
            app.toggle_bluetooth(true);
        }
        app
    }
}

impl AudioRouterApp {
    fn web_state_json(&self) -> Value {
        let devices = self.devices.values().map(|device| {
            let detail = if device.transport == TransportKind::Lan && !self.settings.show_ip {
                "局域网设备".to_string()
            } else {
                device.lan_address.map(|address| address.ip().to_string()).unwrap_or_else(|| device.detail.clone())
            };
            let connected_at = device.connected_at.map(|at| {
                now_millis().saturating_sub(at.elapsed().as_millis() as u64)
            });
            json!({
                "id": device.key,
                "name": device.name,
                "detail": detail,
                "transport": if device.transport == TransportKind::Lan { "lan" } else { "bluetooth" },
                "connected": device.connected,
                "connectedAt": connected_at,
                "sendEnabled": device.send_enabled,
                "receiveEnabled": device.receive_enabled,
                "sendVolume": device.send_volume,
                "receiveVolume": device.receive_volume,
            })
        }).collect::<Vec<_>>();
        json!({
            "captureEnabled": self.send_playback,
            "lanEnabled": self.lan_server.is_some(),
            "bluetoothEnabled": self.bluetooth_server.is_some(),
            "devices": devices,
            "lastAction": self.last_action,
            "preferences": {
                "theme": match self.settings.theme { AppTheme::Light => "light", AppTheme::Dark => "dark", AppTheme::System => "system" },
                "preconnectControls": self.settings.preconnect_controls,
                "showIp": self.settings.show_ip,
                "lanMode": if self.settings.lan_compressed { "compressed" } else { "raw" },
                "compressedRate": self.settings.lan_bitrate_kbps,
                "rawRate": 1536,
                "bluetoothProtocol": match self.settings.bluetooth_preference { PREFERENCE_OPUS => "opus", PREFERENCE_PCM => "pcm", _ => "auto" },
                "buffer": if self.settings.buffer_ms == 0 { json!("custom") } else { json!(self.settings.buffer_ms) },
                "customBuffer": self.settings.custom_buffer_ms,
                "launchOnStartup": self.settings.launch_on_startup,
                "exitBehavior": self.settings.exit_behavior,
                "restoreConnectionState": self.settings.restore_connection_state,
                "autoConnect": self.settings.auto_connect,
                "autoConnectAttempts": self.settings.auto_connect_attempts,
                "autoConnectIntervalSeconds": self.settings.auto_connect_interval_seconds,
            }
        })
    }

    fn handle_web_command(&mut self, command: &Value) {
        let action = command
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match action {
            "capture-toggle" => {
                self.capture_requested = command
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !self.capture_requested {
                    let keys = self
                        .devices
                        .values()
                        .filter(|device| device.connected && device.send_enabled)
                        .map(|device| device.key.clone())
                        .collect::<Vec<_>>();
                    for key in keys {
                        if let Some(device) = self.devices.get_mut(&key) {
                            device.send_enabled = false;
                        }
                        self.send_device_control(&key, ControlMessage::SetReceiveEnabled(false));
                    }
                }
                self.update_capture_state();
            }
            "transport-toggle" => {
                let enabled = command
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match command.get("transport").and_then(Value::as_str) {
                    Some("lan") => self.toggle_lan(enabled),
                    Some("bluetooth") => self.toggle_bluetooth(enabled),
                    _ => {}
                }
            }
            "open-external-url" => {
                if let Some(url) = command.get("url").and_then(Value::as_str) {
                    webview::open_external_url(url);
                }
            }
            "refresh" => match command.get("transport").and_then(Value::as_str) {
                Some("lan") => self.refresh_transport(TransportKind::Lan),
                Some("bluetooth") => self.refresh_transport(TransportKind::Bluetooth),
                _ => {}
            },
            "device-action" => {
                if let Some(key) = command.get("deviceId").and_then(Value::as_str) {
                    match command.get("deviceAction").and_then(Value::as_str) {
                        Some("connect") => self.request_device_connection(key),
                        Some("disconnect") => self.disconnect_device(key),
                        _ => {}
                    }
                }
            }
            "device-switch" => {
                if let Some(key) = command.get("deviceId").and_then(Value::as_str) {
                    let enabled = command
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let control = command
                        .get("control")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let connected = self.devices.get(key).is_some_and(|device| device.connected);
                    if let Some(device) = self.devices.get_mut(key) {
                        if control == "send" {
                            device.send_enabled = enabled;
                        } else if control == "receive" {
                            device.receive_enabled = enabled;
                        }
                    }
                    self.save_device_settings(key);
                    if connected {
                        if control == "send" {
                            self.send_device_control(
                                key,
                                ControlMessage::SetReceiveEnabled(enabled),
                            );
                        }
                        if control == "receive" {
                            self.send_device_control(key, ControlMessage::SetSendEnabled(enabled));
                        }
                        self.update_capture_state();
                    }
                }
            }
            "device-volume" => {
                if let Some(key) = command.get("deviceId").and_then(Value::as_str) {
                    let volume = command
                        .get("volume")
                        .and_then(Value::as_u64)
                        .unwrap_or(100)
                        .min(100) as u8;
                    if command.get("direction").and_then(Value::as_str) == Some("send") {
                        if let Some(device) = self.devices.get_mut(key) {
                            device.send_volume = volume;
                        }
                        self.send_device_control(key, ControlMessage::SetReceiveVolume(volume));
                    } else {
                        if let Some(device) = self.devices.get_mut(key) {
                            device.receive_volume = volume;
                        }
                        self.send_device_control(key, ControlMessage::SetSendVolume(volume));
                    }
                    self.save_device_settings(key);
                }
            }
            "setting" => {
                let previous_settings = self.settings.clone();
                match command.get("key").and_then(Value::as_str) {
                    Some("theme") => {
                        self.settings.theme = match command.get("value").and_then(Value::as_str) {
                            Some("light") => AppTheme::Light,
                            Some("system") => AppTheme::System,
                            _ => AppTheme::Dark,
                        }
                    }
                    Some("preconnectControls") => {
                        self.settings.preconnect_controls = command
                            .get("value")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    }
                    Some("showIp") => {
                        self.settings.show_ip = command
                            .get("value")
                            .and_then(Value::as_bool)
                            .unwrap_or(true)
                    }
                    Some("launchOnStartup") => {
                        self.settings.launch_on_startup = command
                            .get("value")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    }
                    Some("exitBehavior") => {
                        self.settings.exit_behavior =
                            match command.get("value").and_then(Value::as_str) {
                                Some("minimize_to_tray") => "minimize_to_tray".to_string(),
                                _ => "full_exit".to_string(),
                            }
                    }
                    Some("restoreConnectionState") => {
                        self.settings.restore_connection_state = command
                            .get("value")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    }
                    Some("autoConnect") => {
                        self.settings.auto_connect = command
                            .get("value")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        if !self.settings.auto_connect {
                            self.auto_connect_tasks.clear();
                        }
                    }
                    Some("autoConnectAttempts") => {
                        self.settings.auto_connect_attempts = command
                            .get("value")
                            .and_then(Value::as_u64)
                            .unwrap_or(3)
                            .min(5)
                            as u8
                    }
                    Some("autoConnectIntervalSeconds") => {
                        self.settings.auto_connect_interval_seconds =
                            command.get("value").and_then(Value::as_u64).unwrap_or(3) as u32
                    }
                    Some("lanMode") if !self.audio_settings_locked() => {
                        self.settings.lan_compressed =
                            command.get("value").and_then(Value::as_str) != Some("raw")
                    }
                    Some("compressedRate") if !self.audio_settings_locked() => {
                        self.settings.lan_bitrate_kbps = command
                            .get("value")
                            .and_then(Value::as_u64)
                            .unwrap_or(128)
                            .clamp(32, 450)
                            as u32
                    }
                    Some("bluetoothProtocol") if !self.audio_settings_locked() => {
                        self.settings.bluetooth_preference =
                            match command.get("value").and_then(Value::as_str) {
                                Some("opus") => PREFERENCE_OPUS,
                                Some("pcm") => PREFERENCE_PCM,
                                _ => PREFERENCE_AUTO,
                            }
                    }
                    Some("buffer") if !self.audio_settings_locked() => {
                        let value = command.get("value");
                        self.settings.buffer_ms = if value.and_then(Value::as_str) == Some("custom")
                        {
                            0
                        } else {
                            value.and_then(Value::as_u64).unwrap_or(60).clamp(20, 240) as u16
                        }
                    }
                    Some("customBuffer") if !self.audio_settings_locked() => {
                        self.settings.custom_buffer_ms = command
                            .get("value")
                            .and_then(Value::as_u64)
                            .unwrap_or(80)
                            .clamp(20, 240)
                            as u16
                    }
                    _ => {}
                }
                if self.settings != previous_settings {
                    if command.get("key").and_then(Value::as_str) == Some("launchOnStartup") {
                        match self.startup_registration.request(
                            self.settings.launch_on_startup,
                            previous_settings.launch_on_startup,
                        ) {
                            Ok(request_id) => {
                                self.startup_registration_request_id = request_id;
                            }
                            Err(error) => {
                                self.last_action = error;
                                self.settings.launch_on_startup =
                                    previous_settings.launch_on_startup;
                            }
                        }
                    }
                    if self.settings != previous_settings {
                        if let Err(error) = self.settings.save() {
                            self.last_action = error;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn set_send_enabled(&mut self, enabled: bool) {
        self.send_playback = enabled;
        if enabled {
            if self.loopback_capture.is_none() {
                match LoopbackCapture::start() {
                    Ok(capture) => {
                        self.loopback_capture = Some(capture);
                        self.last_action =
                            "Windows WASAPI Loopback 已启动，等待局域网设备".to_string();
                    }
                    Err(error) => {
                        self.send_playback = false;
                        self.last_action = error;
                    }
                }
            }
        } else {
            self.loopback_capture = None;
            self.last_action = "已停止发送本机播放音频".to_string();
        }
    }

    fn update_capture_state(&mut self) {
        let should_capture = self.capture_requested;
        if should_capture != self.send_playback {
            self.set_send_enabled(should_capture);
        }
    }

    fn poll_capture(&mut self) {
        let events = self
            .loopback_capture
            .as_ref()
            .map(|capture| capture.poll())
            .unwrap_or_default();
        let mut capture_failed = false;
        for event in events {
            match event {
                CaptureEvent::Frame(pcm) => {
                    if !self.send_playback {
                        continue;
                    }
                    let payload = pcm;
                    let timestamp = now_millis();
                    let lan_targets = self
                        .devices
                        .values()
                        .filter(|device| {
                            device.transport == TransportKind::Lan
                                && device.connected
                                && device.send_enabled
                        })
                        .filter_map(|device| {
                            device.key.strip_prefix("lan:").and_then(|value| {
                                value
                                    .parse()
                                    .ok()
                                    .map(|address| (address, device.send_volume))
                            })
                        })
                        .collect::<Vec<_>>();
                    if self.lan_server.is_some() {
                        let mut delivered = 0;
                        for (address, volume) in lan_targets {
                            let scaled_payload = audio::apply_volume(&payload, volume);
                            let peer_supports_opus = self
                                .lan_server
                                .as_ref()
                                .is_some_and(|server| server.peer_supports_opus(address));
                            let use_opus = self.settings.lan_compressed && peer_supports_opus;
                            let opus_frame = if use_opus {
                                self.encode_lan_frame(
                                    address,
                                    &scaled_payload,
                                    self.send_sequence,
                                    timestamp,
                                    self.settings.lan_bitrate_kbps,
                                )
                            } else {
                                None
                            };
                            let pcm_frame = audiorouter_core::AudioPacket::new_with_codec(
                                self.send_session_id,
                                self.send_sequence,
                                timestamp,
                                48_000,
                                2,
                                Codec::Pcm16Le,
                                scaled_payload,
                            )
                            .and_then(|packet| packet.encode());
                            if let (Some(server), Ok(pcm_frame)) =
                                (self.lan_server.as_ref(), pcm_frame)
                            {
                                match server.send_audio_to(
                                    address,
                                    self.send_session_id,
                                    self.send_sequence,
                                    timestamp,
                                    48_000,
                                    2,
                                    &pcm_frame[40..],
                                    opus_frame.as_deref(),
                                    self.settings.lan_bitrate_kbps,
                                ) {
                                    Ok(true) => delivered += 1,
                                    Ok(false) => {}
                                    Err(error) => self.last_action = error,
                                }
                            }
                        }
                        if delivered > 0 {
                            self.last_action = format!("已发送音频帧到 {delivered} 台局域网设备");
                        }
                    }
                    let bluetooth_targets = self
                        .devices
                        .values()
                        .filter(|device| {
                            device.transport == TransportKind::Bluetooth
                                && device.connected
                                && device.send_enabled
                        })
                        .filter_map(|device| {
                            device.key.strip_prefix("bt:").and_then(|value| {
                                value.parse().ok().map(|key| (key, device.send_volume))
                            })
                        })
                        .collect::<Vec<_>>();
                    let opus_targets = self
                        .bluetooth_server
                        .as_ref()
                        .map(|server| {
                            bluetooth_targets
                                .iter()
                                .filter_map(|(key, _)| {
                                    let use_opus = match self.settings.bluetooth_preference {
                                        PREFERENCE_OPUS => server.peer_supports_opus(*key),
                                        PREFERENCE_PCM => false,
                                        _ => server.peer_uses_opus(*key),
                                    };
                                    use_opus.then_some(*key)
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let mut delivered = 0;
                    for (key, volume) in bluetooth_targets {
                        let scaled_payload = audio::apply_volume(&payload, volume);
                        let opus_frame = if opus_targets.contains(&key) {
                            self.encode_bluetooth_frame(
                                key,
                                &scaled_payload,
                                self.send_sequence,
                                timestamp,
                            )
                        } else {
                            None
                        };
                        let pcm_frame = audiorouter_core::AudioPacket::new_with_codec(
                            self.send_session_id,
                            self.send_sequence,
                            timestamp,
                            48_000,
                            2,
                            Codec::Pcm16Le,
                            scaled_payload,
                        )
                        .and_then(|packet| packet.encode());
                        if let (Some(server), Ok(pcm_frame)) = (&self.bluetooth_server, pcm_frame) {
                            match server.send_audio_frame_to(key, &pcm_frame, opus_frame.as_deref())
                            {
                                Ok(true) => delivered += 1,
                                Ok(false) => {}
                                Err(error) => self.last_action = error,
                            }
                        }
                    }
                    if delivered > 0 {
                        self.last_action = format!("已发送音频帧到 {delivered} 台蓝牙设备");
                    }
                    self.send_sequence = self.send_sequence.wrapping_add(1);
                }
                CaptureEvent::Error(error) => {
                    self.last_action = error;
                    capture_failed = true;
                }
            }
        }
        if capture_failed {
            self.loopback_capture = None;
            self.send_playback = false;
        }
    }

    fn encode_lan_frame(
        &mut self,
        address: SocketAddr,
        pcm: &[u8],
        sequence: u32,
        timestamp: u64,
        bitrate_kbps: u32,
    ) -> Option<Vec<u8>> {
        let recreate = self
            .lan_opus_encoders
            .get(&address)
            .is_none_or(|encoder| encoder.bitrate_kbps() != bitrate_kbps);
        if recreate {
            match opus_codec::OpusFrameEncoder::new_with_bitrate(2, bitrate_kbps) {
                Ok(encoder) => {
                    self.lan_opus_encoders.insert(address, encoder);
                }
                Err(error) => {
                    self.last_action = error;
                    return None;
                }
            }
        }
        let Some(encoder) = self.lan_opus_encoders.get_mut(&address) else {
            self.last_action = "局域网 Opus 编码器初始化失败".to_string();
            return None;
        };
        let encoded = match encoder.push_pcm16le(pcm) {
            Ok(packets) => packets,
            Err(error) => {
                self.last_action = error;
                return None;
            }
        };
        let Some(payload) = encoded.into_iter().next() else {
            return None;
        };
        audiorouter_core::AudioPacket::new_with_codec(
            self.send_session_id,
            sequence,
            timestamp,
            48_000,
            2,
            Codec::Opus,
            payload,
        )
        .and_then(|packet| packet.encode())
        .map_err(|error| self.last_action = format!("创建局域网 Opus 帧失败：{error}"))
        .ok()
    }

    fn encode_bluetooth_frame(
        &mut self,
        key: usize,
        pcm: &[u8],
        sequence: u32,
        timestamp: u64,
    ) -> Option<Vec<u8>> {
        if !self.bluetooth_opus_encoders.contains_key(&key) {
            match opus_codec::OpusFrameEncoder::new(2) {
                Ok(encoder) => {
                    self.bluetooth_opus_encoders.insert(key, encoder);
                }
                Err(error) => {
                    self.last_action = error;
                    return None;
                }
            }
        }
        let Some(encoder) = self.bluetooth_opus_encoders.get_mut(&key) else {
            self.last_action = "Opus 编码器初始化失败".to_string();
            return None;
        };
        let encoded = match encoder.push_pcm16le(pcm) {
            Ok(packets) => packets,
            Err(error) => {
                self.last_action = error;
                return None;
            }
        };
        let Some(payload) = encoded.into_iter().next() else {
            return None;
        };
        audiorouter_core::AudioPacket::new_with_codec(
            self.send_session_id,
            sequence,
            timestamp,
            48_000,
            2,
            Codec::Opus,
            payload,
        )
        .and_then(|packet| packet.encode())
        .map_err(|error| self.last_action = format!("创建蓝牙 Opus 帧失败：{error}"))
        .ok()
    }

    fn decode_lan_packet(
        &mut self,
        address: SocketAddr,
        packet: audiorouter_core::AudioPacket,
    ) -> Result<audiorouter_core::AudioPacket, String> {
        if packet.codec == Codec::Pcm16Le {
            return Ok(packet);
        }
        if packet.sample_rate != 48_000 {
            return Err(format!(
                "不支持的局域网 Opus 采样率：{} Hz",
                packet.sample_rate
            ));
        }
        let decoder_key = (address, packet.session_id, packet.channels);
        if !self.lan_opus_decoders.contains_key(&decoder_key) {
            let decoder = opus_codec::OpusFrameDecoder::new(packet.channels)?;
            self.lan_opus_decoders.insert(decoder_key, decoder);
        }
        let decoder = self
            .lan_opus_decoders
            .get_mut(&decoder_key)
            .ok_or_else(|| "局域网 Opus 解码器初始化失败".to_string())?;
        let pcm = decoder.decode(&packet.payload)?;
        audiorouter_core::AudioPacket::new_with_codec(
            packet.session_id,
            packet.sequence,
            packet.timestamp_ms,
            packet.sample_rate,
            packet.channels,
            Codec::Pcm16Le,
            pcm,
        )
        .map_err(|error| format!("创建局域网 PCM 播放帧失败：{error}"))
    }

    fn decode_bluetooth_packet(
        &mut self,
        packet: audiorouter_core::AudioPacket,
    ) -> Result<audiorouter_core::AudioPacket, String> {
        if packet.codec == Codec::Pcm16Le {
            return Ok(packet);
        }
        if packet.sample_rate != 48_000 {
            return Err(format!("不支持的 Opus 采样率：{} Hz", packet.sample_rate));
        }
        let decoder_key = (packet.session_id, packet.channels);
        if !self.bluetooth_opus_decoders.contains_key(&decoder_key) {
            let decoder = opus_codec::OpusFrameDecoder::new(packet.channels)?;
            self.bluetooth_opus_decoders.insert(decoder_key, decoder);
        }
        let decoder = self
            .bluetooth_opus_decoders
            .get_mut(&decoder_key)
            .ok_or_else(|| "Opus 解码器初始化失败".to_string())?;
        let pcm = decoder.decode(&packet.payload)?;
        audiorouter_core::AudioPacket::new_with_codec(
            packet.session_id,
            packet.sequence,
            packet.timestamp_ms,
            packet.sample_rate,
            packet.channels,
            Codec::Pcm16Le,
            pcm,
        )
        .map_err(|error| format!("创建蓝牙 PCM 播放帧失败：{error}"))
    }

    fn enqueue_playback(&mut self, packet: AudioPacket, volume: u8) {
        match self.playback_tx.push_latest(
            (packet, volume_to_gain(volume)),
            (self.active_buffer_ms() as usize).div_ceil(20).clamp(1, 12),
        ) {
            Ok(true) => {
                self.playback_dropped_packets += 1;
                if self.playback_dropped_packets == 1 || self.playback_dropped_packets % 25 == 0 {
                    self.last_action = format!(
                        "为保持实时播放，已跳过 {} 个过期音频帧",
                        self.playback_dropped_packets
                    );
                }
            }
            Ok(false) => {}
            Err(()) => {
                self.last_action = "音频播放队列不可用".to_string();
            }
        }
    }
}

impl AudioRouterApp {
    fn active_buffer_ms(&self) -> u16 {
        match self.settings.buffer_ms {
            0 => self.settings.custom_buffer_ms.clamp(20, 240),
            value => value,
        }
    }

    fn device_key(transport: TransportKind, id: impl std::fmt::Display) -> String {
        match transport {
            TransportKind::Lan => format!("lan:{id}"),
            TransportKind::Bluetooth => format!("bt:{id}"),
        }
    }

    fn ensure_device(
        &mut self,
        key: String,
        transport: TransportKind,
        name: impl Into<String>,
        detail: impl Into<String>,
        connected: bool,
    ) {
        let name = name.into();
        let detail = detail.into();
        let existing_name = self.devices.get(&key).map(|device| device.name.clone());
        let name = Self::friendly_device_name(&name, existing_name.as_deref());
        let remembered = self
            .settings
            .device_preferences
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let device = self
            .devices
            .entry(key.clone())
            .or_insert_with(|| DeviceCard {
                key,
                transport,
                name: name.clone(),
                detail: detail.clone(),
                connected,
                connected_at: connected.then(Instant::now),
                send_enabled: remembered.send_enabled,
                receive_enabled: remembered.receive_enabled,
                send_volume: remembered.send_volume,
                receive_volume: remembered.receive_volume,
                lan_address: None,
                bluetooth_address: None,
            });
        if !name.is_empty() && !Self::is_generic_device_name(&name) {
            device.name = name;
        }
        if !detail.is_empty() {
            device.detail = detail;
        }
        if connected && !device.connected {
            device.connected_at = Some(Instant::now());
        } else if !connected {
            device.connected_at = None;
        }
        device.connected = connected;
    }

    fn is_generic_device_name(name: &str) -> bool {
        matches!(
            name.trim(),
            "" | "AudioRouter Android 16" | "Android 设备" | "蓝牙设备" | "局域网设备"
        )
    }

    fn friendly_device_name(name: &str, fallback: Option<&str>) -> String {
        let name = name.trim();
        if Self::is_generic_device_name(name) {
            if let Some(fallback) = fallback.filter(|value| !Self::is_generic_device_name(value)) {
                return fallback.trim().to_string();
            }
            return if name == "AudioRouter Android 16" {
                "Android 设备".to_string()
            } else {
                name.to_string()
            };
        }
        name.to_string()
    }

    fn promote_device(
        &mut self,
        placeholder_key: &str,
        key: String,
        transport: TransportKind,
        detail: impl Into<String>,
        connected: bool,
    ) {
        let detail = detail.into();
        if let Some(remembered) = self.settings.device_preferences.remove(placeholder_key) {
            let remembered_key = if transport == TransportKind::Bluetooth {
                self.devices
                    .get(placeholder_key)
                    .and_then(|device| device.bluetooth_address)
                    .map(|address| format!("bt-address:{address:012X}"))
                    .unwrap_or_else(|| key.clone())
            } else {
                key.clone()
            };
            self.settings
                .device_preferences
                .entry(remembered_key)
                .or_insert(remembered);
        }
        if let Some(mut device) = self.devices.remove(placeholder_key) {
            device.key = key.clone();
            device.transport = transport;
            device.name = Self::friendly_device_name(&device.name, None);
            device.detail = detail;
            device.connected = connected;
            self.devices.insert(key, device);
        } else {
            let name = match transport {
                TransportKind::Lan => "局域网设备",
                TransportKind::Bluetooth => "蓝牙设备",
            };
            self.ensure_device(key, transport, name, detail, connected);
        }
    }

    fn request_device_connection(&mut self, key: &str) {
        let target = self.auto_target_id(key);
        self.suppressed_auto_targets.remove(&target);
        if self.settings.auto_connect {
            self.auto_connect_tasks
                .insert(target, retry::RetryTask::new(Instant::now()));
            self.process_auto_connect_tasks();
        } else {
            self.connect_device_once(key);
        }
    }

    fn connect_device_once(&mut self, key: &str) {
        if let Some(value) = key
            .strip_prefix("lan-discovered:")
            .or_else(|| key.strip_prefix("lan:"))
        {
            let Ok(address) = value.parse::<SocketAddr>() else {
                return;
            };
            let Some(server) = self.lan_server.as_ref() else {
                self.last_action = "请先启用局域网连接".to_string();
                return;
            };
            match server.connect(address) {
                Ok(()) => self.last_action = "正在连接局域网设备".to_string(),
                Err(error) => self.last_action = error,
            }
            return;
        }
        if let Some(value) = key
            .strip_prefix("bt-paired:")
            .or_else(|| key.strip_prefix("bt-address:"))
        {
            let Ok(address) = u64::from_str_radix(value, 16) else {
                return;
            };
            let Some(server) = self.bluetooth_server.as_ref() else {
                self.last_action = "请先启用蓝牙连接".to_string();
                return;
            };
            match server.connect(address) {
                Ok(()) => self.last_action = "正在连接蓝牙设备".to_string(),
                Err(error) => self.last_action = error,
            }
            return;
        }
        if key.starts_with("bt:") {
            let address = self
                .devices
                .get(key)
                .and_then(|device| device.bluetooth_address);
            let Some(address) = address else {
                return;
            };
            let Some(server) = self.bluetooth_server.as_ref() else {
                self.last_action = "请先启用蓝牙连接".to_string();
                return;
            };
            match server.connect(address) {
                Ok(()) => self.last_action = "正在连接蓝牙设备".to_string(),
                Err(error) => self.last_action = error,
            }
        }
    }

    fn mark_transport_disconnected(&mut self, transport: TransportKind) {
        for device in self.devices.values_mut() {
            if device.transport == transport {
                device.connected = false;
            }
        }
    }

    fn remove_lan_duplicates(&mut self, address: SocketAddr, keep_key: Option<&str>) {
        let ip = address.ip();
        self.devices.retain(|key, device| {
            if Some(key.as_str()) == keep_key || device.transport != TransportKind::Lan {
                return true;
            }
            let same_ip = device.lan_address.is_some_and(|known| known.ip() == ip);
            !same_ip
        });
    }

    fn send_device_control(&mut self, key: &str, message: ControlMessage) {
        let Some(device) = self.devices.get(key) else {
            return;
        };
        let result = match device.transport {
            TransportKind::Lan => {
                let Some(address) = key
                    .strip_prefix("lan:")
                    .and_then(|value| value.parse().ok())
                else {
                    return;
                };
                self.lan_server
                    .as_ref()
                    .ok_or_else(|| "局域网主机未启动".to_string())
                    .and_then(|server| server.send_control(address, message))
            }
            TransportKind::Bluetooth => {
                let Some(peer_key) = key.strip_prefix("bt:").and_then(|value| value.parse().ok())
                else {
                    return;
                };
                self.bluetooth_server
                    .as_ref()
                    .ok_or_else(|| "蓝牙主机未启动".to_string())
                    .and_then(|server| server.send_control(peer_key, message))
            }
        };
        if let Err(error) = result {
            self.last_action = error;
        }
    }

    fn disconnect_device(&mut self, key: &str) {
        let Some(device) = self.devices.get(key) else {
            return;
        };
        let result = match device.transport {
            TransportKind::Lan => key
                .strip_prefix("lan:")
                .and_then(|value| value.parse::<SocketAddr>().ok())
                .ok_or_else(|| "局域网设备地址无效".to_string())
                .and_then(|address| {
                    self.lan_server
                        .as_ref()
                        .ok_or_else(|| "局域网主机未启动".to_string())?
                        .disconnect(address)
                }),
            TransportKind::Bluetooth => key
                .strip_prefix("bt:")
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| "蓝牙设备标识无效".to_string())
                .and_then(|peer| {
                    self.bluetooth_server
                        .as_ref()
                        .ok_or_else(|| "蓝牙主机未启动".to_string())?
                        .disconnect(peer)
                }),
        };
        if let Err(error) = result {
            self.last_action = error;
        }
        if let Some(device) = self.devices.get_mut(key) {
            device.connected = false;
            device.connected_at = None;
        }
        self.forget_auto_target(key);
        self.update_capture_state();
    }

    fn sync_device_controls(&mut self, key: &str) {
        let Some((send_enabled, receive_enabled, send_volume, receive_volume)) =
            self.devices.get(key).map(|device| {
                (
                    device.send_enabled,
                    device.receive_enabled,
                    device.send_volume,
                    device.receive_volume,
                )
            })
        else {
            return;
        };
        self.send_device_control(key, ControlMessage::SetReceiveEnabled(send_enabled));
        self.send_device_control(key, ControlMessage::SetSendEnabled(receive_enabled));
        self.send_device_control(key, ControlMessage::SetReceiveVolume(send_volume));
        self.send_device_control(key, ControlMessage::SetSendVolume(receive_volume));
    }

    fn device_preference_key(&self, key: &str) -> String {
        let Some(device) = self.devices.get(key) else {
            return key.to_string();
        };
        match device.transport {
            TransportKind::Lan => key.to_string(),
            TransportKind::Bluetooth => device
                .bluetooth_address
                .map(|address| format!("bt-address:{address:012X}"))
                .unwrap_or_else(|| key.to_string()),
        }
    }

    fn restore_device_settings(&mut self, key: &str) {
        let preference_key = self.device_preference_key(key);
        let remembered = self
            .settings
            .device_preferences
            .get(&preference_key)
            .cloned()
            .or_else(|| self.settings.device_preferences.get(key).cloned());
        let Some(remembered) = remembered else {
            return;
        };
        if let Some(device) = self.devices.get_mut(key) {
            device.send_enabled = remembered.send_enabled;
            device.receive_enabled = remembered.receive_enabled;
            device.send_volume = remembered.send_volume;
            device.receive_volume = remembered.receive_volume;
        }
    }

    fn save_device_settings(&mut self, key: &str) {
        let preference_key = self.device_preference_key(key);
        let Some(device) = self.devices.get(key) else {
            return;
        };
        self.settings.device_preferences.insert(
            preference_key,
            DeviceSettings {
                send_enabled: device.send_enabled,
                receive_enabled: device.receive_enabled,
                send_volume: device.send_volume,
                receive_volume: device.receive_volume,
            },
        );
        if let Err(error) = self.settings.save() {
            self.last_action = error;
        }
    }

    fn toggle_lan(&mut self, enabled: bool) {
        if enabled {
            if self.lan_server.is_some() {
                return;
            }
            match LanServer::start("0.0.0.0:40400".parse().expect("valid LAN bind address")) {
                Ok(server) => {
                    self.lan_server = Some(server);
                    self.settings.lan_enabled = true;
                    let _ = self.settings.save();
                    self.last_action = "局域网主机已启动，等待设备连接".to_string();
                }
                Err(error) => {
                    self.last_action = lan::startup_error_message(&error);
                    self.transport_error = Some(self.last_action.clone());
                }
            }
        } else {
            self.settings.lan_enabled = false;
            let _ = self.settings.save();
            self.auto_connect_tasks
                .retain(|key, _| !key.starts_with("lan:") && !key.starts_with("lan-discovered:"));
            self.lan_server = None;
            self.lan_opus_encoders.clear();
            self.lan_opus_decoders.clear();
            self.mark_transport_disconnected(TransportKind::Lan);
            self.last_action = "局域网连接已关闭".to_string();
            self.update_capture_state();
        }
    }

    fn toggle_bluetooth(&mut self, enabled: bool) {
        if enabled {
            if self.bluetooth_server.is_some() {
                return;
            }
            match BluetoothServer::start() {
                Ok(server) => {
                    self.bluetooth_server = Some(server);
                    self.settings.bluetooth_enabled = true;
                    let _ = self.settings.save();
                    self.last_action = "蓝牙主机已启动，等待设备连接".to_string();
                }
                Err(error) => {
                    self.last_action = format!("蓝牙启动失败：{error}");
                    self.transport_error = Some(self.last_action.clone());
                }
            }
        } else {
            self.settings.bluetooth_enabled = false;
            let _ = self.settings.save();
            self.auto_connect_tasks
                .retain(|key, _| !key.starts_with("bt:") && !key.starts_with("bt-paired:"));
            self.bluetooth_server = None;
            self.bluetooth_opus_encoders.clear();
            self.mark_transport_disconnected(TransportKind::Bluetooth);
            self.last_action = "蓝牙连接已关闭".to_string();
            self.update_capture_state();
        }
    }

    fn transport_active(&self, transport: TransportKind) -> bool {
        match transport {
            TransportKind::Lan => self.lan_server.is_some(),
            TransportKind::Bluetooth => self.bluetooth_server.is_some(),
        }
    }

    fn auto_target_id(&self, key: &str) -> String {
        let Some(device) = self.devices.get(key) else {
            return key.to_string();
        };
        match device.transport {
            TransportKind::Lan => device
                .lan_address
                .map(|address| format!("lan:{address}"))
                .unwrap_or_else(|| key.to_string()),
            TransportKind::Bluetooth => device
                .bluetooth_address
                .map(|address| format!("bt-address:{address:012X}"))
                .unwrap_or_else(|| key.to_string()),
        }
    }

    fn is_auto_target(&self, key: &str) -> bool {
        let target = self.auto_target_id(key);
        self.settings
            .auto_connect_targets
            .iter()
            .any(|candidate| candidate == &target || candidate == key)
    }

    fn device_key_for_auto_target(&self, target: &str) -> Option<String> {
        self.devices.keys().find_map(|key| {
            (self.auto_target_id(key) == target || key == target).then_some(key.clone())
        })
    }

    fn remember_auto_target(&mut self, key: &str) {
        let target = self.auto_target_id(key);
        if !self.settings.auto_connect_targets.contains(&target) {
            self.settings.auto_connect_targets.push(target);
            let _ = self.settings.save();
        }
    }

    fn forget_auto_target(&mut self, key: &str) {
        let target = self.auto_target_id(key);
        self.suppressed_auto_targets.insert(target.clone());
        self.settings
            .auto_connect_targets
            .retain(|candidate| candidate != &target && candidate != key);
        self.auto_connect_tasks.remove(&target);
        let _ = self.settings.save();
    }

    fn schedule_auto_connect(&mut self, key: &str) {
        let target = self.auto_target_id(key);
        if !self.settings.auto_connect
            || !self.is_auto_target(key)
            || self.suppressed_auto_targets.contains(&target)
            || self.devices.get(key).is_some_and(|device| device.connected)
        {
            return;
        }
        self.auto_connect_tasks
            .entry(target)
            .or_insert_with(|| retry::RetryTask::new(Instant::now()));
    }

    fn fail_auto_connect(&mut self, key: &str) {
        let target = self.auto_target_id(key);
        if let Some(task) = self.auto_connect_tasks.get_mut(&target) {
            task.failed(
                Instant::now(),
                Duration::from_secs(self.settings.auto_connect_interval_seconds as u64),
            );
        }
    }

    fn process_auto_connect_tasks(&mut self) {
        if !self.settings.auto_connect {
            self.auto_connect_tasks.clear();
            return;
        }
        let now = Instant::now();
        let due = self.auto_connect_tasks.keys().cloned().collect::<Vec<_>>();
        for target in due {
            let Some(key) = self.device_key_for_auto_target(&target) else {
                continue;
            };
            if self
                .devices
                .get(&key)
                .is_some_and(|device| device.connected)
            {
                self.auto_connect_tasks.remove(&target);
                continue;
            }
            let interval = Duration::from_secs(self.settings.auto_connect_interval_seconds as u64);
            if let Some(task) = self.auto_connect_tasks.get_mut(&target) {
                task.timed_out(now, interval);
            }
            let should_attempt = self
                .auto_connect_tasks
                .get_mut(&target)
                .is_some_and(|task| {
                    task.begin(
                        now,
                        self.settings.auto_connect_attempts,
                        Duration::from_secs(5),
                    )
                });
            if should_attempt {
                self.connect_device_once(&key);
            }
        }
    }

    fn refresh_transport(&mut self, transport: TransportKind) {
        if !self.transport_active(transport) {
            return;
        }
        match transport {
            TransportKind::Lan => {
                self.lan_refresh_until = Some(Instant::now() + Duration::from_millis(900));
                self.last_action = "正在搜索局域网设备…".to_string();
            }
            TransportKind::Bluetooth => {
                self.bluetooth_refresh_until = Some(Instant::now() + Duration::from_millis(900));
                self.bluetooth_scan_at = Instant::now() - Duration::from_secs(1);
                self.last_action = "正在刷新已配对设备…".to_string();
            }
        }
    }

    fn refresh_in_progress(&self, transport: TransportKind) -> bool {
        match transport {
            TransportKind::Lan => self
                .lan_refresh_until
                .is_some_and(|until| until > Instant::now()),
            TransportKind::Bluetooth => self
                .bluetooth_refresh_until
                .is_some_and(|until| until > Instant::now()),
        }
    }

    fn panel_frame(ui: &egui::Ui, fill: egui::Color32, stroke: egui::Color32) -> egui::Frame {
        egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(1.0_f32, stroke))
            .rounding(egui::Rounding::same(12.0))
            .inner_margin(egui::Margin::same(16.0))
    }

    fn draw_switch(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
        let response = ui.add(
            egui::Button::new(
                egui::RichText::new(if *value { "●  开" } else { "○  关" }).size(12.0),
            )
            .selected(*value),
        );
        if response.clicked() {
            *value = !*value;
        }
        response.on_hover_text(label)
    }

    fn render_transport_panel(&mut self, ui: &mut egui::Ui, transport: TransportKind) {
        let accent = transport.accent();
        let active = self.transport_active(transport);
        let heading = match transport {
            TransportKind::Lan => "▣  局域网连接",
            TransportKind::Bluetooth => "ᛒ  蓝牙连接",
        };
        let subtitle = match transport {
            TransportKind::Lan => "连接同一网络中的 Android 设备",
            TransportKind::Bluetooth => "连接电脑已配对的 Android 设备",
        };
        let surface = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(24, 38, 34)
        } else {
            egui::Color32::from_rgb(255, 255, 255)
        };
        let border = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(59, 85, 77)
        } else {
            egui::Color32::from_rgb(193, 209, 199)
        };
        Self::panel_frame(ui, surface, border).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(heading)
                        .color(accent)
                        .size(18.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(egui::RichText::new(subtitle).color(ui.visuals().weak_text_color()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut enabled = active;
                    if Self::draw_switch(ui, &mut enabled, if active { "关闭" } else { "开启" })
                        .changed()
                    {
                        match transport {
                            TransportKind::Lan => self.toggle_lan(enabled),
                            TransportKind::Bluetooth => self.toggle_bluetooth(enabled),
                        }
                    }
                });
            });
            ui.add_space(8.0);
            if !active {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().weak_text_color(), "●");
                    ui.label(
                        egui::RichText::new(match transport {
                            TransportKind::Lan => "开启后搜索局域网设备",
                            TransportKind::Bluetooth => "开启后显示已配对设备",
                        })
                        .color(ui.visuals().weak_text_color()),
                    );
                });
                return;
            }
            ui.separator();
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(match transport {
                            TransportKind::Lan => "局域网内的设备",
                            TransportKind::Bluetooth => "已配对的设备",
                        })
                        .strong(),
                    );
                    ui.label(
                        egui::RichText::new(match transport {
                            TransportKind::Lan => "开启后自动发现同一网络设备",
                            TransportKind::Bluetooth => "显示 Windows 已配对设备",
                        })
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                });
                let count = self
                    .devices
                    .values()
                    .filter(|d| d.transport == transport)
                    .count();
                ui.label(
                    egui::RichText::new(format!("{count} 台"))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                let refreshing = self.refresh_in_progress(transport);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !refreshing,
                            egui::Button::new(
                                egui::RichText::new(if refreshing { "⟳" } else { "⟳" }).size(18.0),
                            ),
                        )
                        .clicked()
                    {
                        self.refresh_transport(transport);
                    }
                });
            });
            ui.add_space(9.0);
            let mut keys = self
                .devices
                .values()
                .filter(|d| d.transport == transport)
                .map(|d| d.key.clone())
                .collect::<Vec<_>>();
            keys.sort_by_key(|key| {
                self.devices
                    .get(key)
                    .map(|d| (!d.connected, d.name.clone()))
            });
            egui::ScrollArea::vertical()
                .id_salt(if transport == TransportKind::Lan {
                    "lan-device-list"
                } else {
                    "bluetooth-device-list"
                })
                .max_height(450.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if keys.is_empty() {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 130.0),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                ui.label(
                                    egui::RichText::new(if transport == TransportKind::Lan {
                                        "暂未发现局域网设备"
                                    } else {
                                        "没有已配对设备"
                                    })
                                    .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(if transport == TransportKind::Lan {
                                        "确认手机与电脑处于同一网络后刷新"
                                    } else {
                                        "请先在 Windows 蓝牙设置中配对手机"
                                    })
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                                );
                            },
                        );
                    }
                    for key in keys {
                        self.render_device_card(ui, &key, accent);
                        ui.add_space(8.0);
                    }
                });
        });
    }

    fn render_device_card(&mut self, ui: &mut egui::Ui, key: &str, accent: egui::Color32) {
        let Some(device) = self.devices.get_mut(key) else {
            return;
        };
        let connected = device.connected;
        let show_controls = connected || self.settings.preconnect_controls;
        let surface = if ui.visuals().dark_mode {
            if connected {
                egui::Color32::from_rgb(28, 48, 42)
            } else {
                egui::Color32::from_rgb(20, 31, 28)
            }
        } else {
            if connected {
                egui::Color32::from_rgb(237, 246, 240)
            } else {
                egui::Color32::from_rgb(246, 248, 246)
            }
        };
        let border = if connected {
            accent
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke.color
        };
        let mut request_connection = false;
        let mut disconnect_requested = false;
        let mut send_enabled_changed = false;
        let mut receive_enabled_changed = false;
        let mut send_volume_changed = false;
        let mut receive_volume_changed = false;
        Self::panel_frame(ui, surface, border).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(if device.transport == TransportKind::Lan {
                        "▣"
                    } else {
                        "ᛒ"
                    })
                    .size(18.0)
                    .color(accent),
                );
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&device.name).strong());
                    let detail = if device.transport == TransportKind::Lan && !self.settings.show_ip
                    {
                        "局域网设备".to_string()
                    } else {
                        device
                            .lan_address
                            .map(|address| address.ip().to_string())
                            .unwrap_or_else(|| device.detail.clone())
                    };
                    ui.label(
                        egui::RichText::new(detail)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if connected {
                        if ui.button("断开").clicked() {
                            disconnect_requested = true;
                        }
                        if let Some(at) = device.connected_at {
                            ui.label(
                                egui::RichText::new(format!(
                                    "在线 {}",
                                    format_connection_duration(at.elapsed())
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        }
                        ui.colored_label(accent, "● 已连接");
                    } else {
                        ui.label(
                            egui::RichText::new("未连接")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        if Self::can_request_connection(device) && ui.button("连接").clicked() {
                            request_connection = true;
                        }
                    }
                });
            });
            if show_controls {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if Self::draw_switch(ui, &mut device.send_enabled, "发送音频").changed() {
                        send_enabled_changed = true;
                    }
                    ui.label(egui::RichText::new("发送音频").strong());
                    ui.label(
                        egui::RichText::new("电脑播放声音 → 此设备")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    let slider = egui::Slider::new(&mut device.send_volume, 0..=100)
                        .show_value(false)
                        .trailing_fill(true);
                    if ui.add_enabled(device.send_enabled, slider).changed() {
                        send_volume_changed = true;
                    }
                    ui.label(format!("{}%", device.send_volume));
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if Self::draw_switch(ui, &mut device.receive_enabled, "接收音频").changed()
                    {
                        receive_enabled_changed = true;
                    }
                    ui.label(egui::RichText::new("接收音频").strong());
                    ui.label(
                        egui::RichText::new("此设备声音 → 电脑播放设备")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    let slider = egui::Slider::new(&mut device.receive_volume, 0..=100)
                        .show_value(false)
                        .trailing_fill(true);
                    if ui.add_enabled(device.receive_enabled, slider).changed() {
                        receive_volume_changed = true;
                    }
                    ui.label(format!("{}%", device.receive_volume));
                });
            }
        });
        if request_connection {
            self.request_device_connection(key);
        }
        if disconnect_requested {
            self.disconnect_device(key);
        }
        if receive_enabled_changed && connected {
            let value = self
                .devices
                .get(key)
                .map(|d| d.receive_enabled)
                .unwrap_or(false);
            self.send_device_control(key, ControlMessage::SetSendEnabled(value));
            self.update_capture_state();
        }
        if send_enabled_changed && connected {
            let value = self
                .devices
                .get(key)
                .map(|d| d.send_enabled)
                .unwrap_or(false);
            self.send_device_control(key, ControlMessage::SetReceiveEnabled(value));
            self.update_capture_state();
        }
        if receive_volume_changed {
            let value = self
                .devices
                .get(key)
                .map(|d| d.receive_volume)
                .unwrap_or(100);
            self.send_device_control(key, ControlMessage::SetSendVolume(value));
        }
        if send_volume_changed {
            let value = self.devices.get(key).map(|d| d.send_volume).unwrap_or(100);
            self.send_device_control(key, ControlMessage::SetReceiveVolume(value));
        }
    }

    fn can_request_connection(device: &DeviceCard) -> bool {
        device.key.starts_with("lan-discovered:")
            || device.key.starts_with("bt-paired:")
            || device.lan_address.is_some()
            || device.bluetooth_address.is_some()
    }

    fn audio_settings_locked(&self) -> bool {
        self.send_playback || self.devices.values().any(|device| device.connected)
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        let locked = self.audio_settings_locked();
        ui.horizontal(|ui| {
            if ui.button("‹").on_hover_text("返回主页").clicked() {
                self.settings_open = false;
                self.settings_category = None;
            }
            ui.heading(if self.settings_category.is_some() {
                "设置"
            } else {
                "设置"
            });
        });
        ui.add_space(18.0);
        let card_fill = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(24, 38, 34)
        } else {
            egui::Color32::WHITE
        };
        let card_border = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(59, 85, 77)
        } else {
            egui::Color32::from_rgb(193, 209, 199)
        };

        let Some(category) = self.settings_category else {
            for (category, icon, title, detail) in [
                (SettingsCategory::Appearance, "◐", "外观", "主题与显示方式"),
                (
                    SettingsCategory::Personalization,
                    "✦",
                    "个性化",
                    "连接前控制与设备信息",
                ),
                (
                    SettingsCategory::Transport,
                    "⇄",
                    "音频传输",
                    "局域网与蓝牙传输方式",
                ),
                (
                    SettingsCategory::Buffer,
                    "◌",
                    "音频缓冲",
                    "缓冲数值越高，传输越稳定，延迟越高",
                ),
            ] {
                let unavailable = locked
                    && matches!(
                        category,
                        SettingsCategory::Transport | SettingsCategory::Buffer
                    );
                Self::panel_frame(
                    ui,
                    card_fill,
                    if unavailable {
                        ui.visuals().weak_text_color()
                    } else {
                        card_border
                    },
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(icon)
                                .size(20.0)
                                .color(egui::Color32::from_rgb(143, 221, 195)),
                        );
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(title).strong());
                            ui.label(
                                egui::RichText::new(detail)
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let response = ui.add_enabled(!unavailable, egui::Button::new("›"));
                            if response.clicked() {
                                self.settings_category = Some(category);
                            }
                        });
                    });
                });
                if unavailable
                    && ui
                        .interact(ui.min_rect(), ui.id().with(title), egui::Sense::click())
                        .clicked()
                {
                    self.show_settings_lock = true;
                }
                ui.add_space(10.0);
            }
            return;
        };

        ui.horizontal(|ui| {
            if ui.button("‹").on_hover_text("返回设置").clicked() {
                self.settings_category = None;
            }
            ui.heading(match category {
                SettingsCategory::Appearance => "外观",
                SettingsCategory::Personalization => "个性化",
                SettingsCategory::Transport => "音频传输",
                SettingsCategory::Buffer => "音频缓冲",
            });
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical().show(ui, |ui| match category {
            SettingsCategory::Appearance => {
                Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                    ui.label(egui::RichText::new("主题").strong());
                    ui.add_space(8.0);
                    for (value, label, description) in [
                        (AppTheme::Light, "浅色", "使用浅色界面"),
                        (AppTheme::Dark, "深色", "使用深色界面"),
                        (AppTheme::System, "跟随系统", "跟随电脑系统的外观设置"),
                    ] {
                        let selected = self.settings.theme == value;
                        if ui
                            .add(
                                egui::Button::new(format!(
                                    "{}  {}\n{}",
                                    if selected { "●" } else { "○" },
                                    label,
                                    description
                                ))
                                .selected(selected),
                            )
                            .clicked()
                        {
                            self.settings.theme = value;
                        }
                        ui.add_space(6.0);
                    }
                });
            }
            SettingsCategory::Personalization => {
                for (value, title, detail) in [
                    (
                        true,
                        "连接前显示音频控制选项",
                        "开启后，连接设备前也可以提前调整发送、接收和音量设置。",
                    ),
                    (
                        false,
                        "显示 IP 地址",
                        "在局域网设备名称下显示设备的 IP 地址。",
                    ),
                ] {
                    Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(title).strong());
                                ui.label(
                                    egui::RichText::new(detail)
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let target = if value {
                                        &mut self.settings.preconnect_controls
                                    } else {
                                        &mut self.settings.show_ip
                                    };
                                    Self::draw_switch(ui, target, title);
                                },
                            );
                        });
                    });
                    ui.add_space(10.0);
                }
                Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                    let mut value = self.settings.restore_connection_state;
                    if ui
                        .checkbox(&mut value, "应用上次关闭时的连接状态")
                        .on_hover_text("下次启动时恢复上次关闭前已开启的局域网和蓝牙连接")
                        .changed()
                    {
                        self.settings.restore_connection_state = value;
                        let _ = self.settings.save();
                    }
                    ui.label(
                        egui::RichText::new("下次启动时恢复局域网和蓝牙连接开关状态")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                ui.add_space(10.0);
                Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                    let mut value = self.settings.auto_connect;
                    if ui
                        .checkbox(&mut value, "自动尝试连接")
                        .on_hover_text("设备连接失败时，按下面的次数和间隔自动重试")
                        .changed()
                    {
                        self.settings.auto_connect = value;
                        let _ = self.settings.save();
                    }
                    ui.label(
                        egui::RichText::new("开启后自动重试上次成功连接过的设备")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    if self.settings.auto_connect {
                        ui.add_space(8.0);
                        egui::ComboBox::from_id_salt("auto-connect-attempts")
                            .selected_text(if self.settings.auto_connect_attempts == 0 {
                                "持续尝试"
                            } else {
                                "选择尝试次数"
                            })
                            .show_ui(ui, |ui| {
                                for value in [1, 2, 3, 4, 5, 0] {
                                    let label = if value == 0 {
                                        "持续尝试".to_string()
                                    } else {
                                        format!("{value} 次")
                                    };
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.auto_connect_attempts,
                                            value,
                                            label,
                                        )
                                        .changed()
                                    {
                                        let _ = self.settings.save();
                                    }
                                }
                            });
                        egui::ComboBox::from_id_salt("auto-connect-interval")
                            .selected_text(format!(
                                "{} 秒",
                                self.settings.auto_connect_interval_seconds
                            ))
                            .show_ui(ui, |ui| {
                                for value in [1, 2, 3, 5, 10, 30] {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.auto_connect_interval_seconds,
                                            value,
                                            format!("{value} 秒"),
                                        )
                                        .changed()
                                    {
                                        let _ = self.settings.save();
                                    }
                                }
                            });
                    }
                });
            }
            SettingsCategory::Transport => {
                if locked {
                    ui.colored_label(ui.visuals().weak_text_color(), "请关闭传输后再调整设置");
                }
                Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                    ui.label(egui::RichText::new("局域网连接").strong());
                    ui.add_space(8.0);
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.radio_value(&mut self.settings.lan_compressed, true, "经过压缩");
                        ui.radio_value(&mut self.settings.lan_compressed, false, "未经压缩");
                    });
                    if self.settings.lan_compressed {
                        ui.label(
                            egui::RichText::new("压缩后传输速率")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        for bitrate in [64, 128, 256, 450] {
                            ui.radio_value(
                                &mut self.settings.lan_bitrate_kbps,
                                bitrate,
                                format!("{bitrate} kb/s"),
                            );
                        }
                    } else {
                        ui.label("未经压缩：1,536 kb/s · 48 kHz · 16 位 · 双声道");
                    }
                });
                ui.add_space(10.0);
                Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                    ui.label(egui::RichText::new("蓝牙连接").strong());
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.radio_value(
                            &mut self.settings.bluetooth_preference,
                            PREFERENCE_AUTO,
                            "自动",
                        );
                        ui.radio_value(
                            &mut self.settings.bluetooth_preference,
                            PREFERENCE_OPUS,
                            "Opus",
                        );
                        ui.radio_value(
                            &mut self.settings.bluetooth_preference,
                            PREFERENCE_PCM,
                            "PCM",
                        );
                    });
                });
            }
            SettingsCategory::Buffer => {
                Self::panel_frame(ui, card_fill, card_border).show(ui, |ui| {
                    ui.label(egui::RichText::new("音频缓冲").strong());
                    ui.label(
                        egui::RichText::new("缓冲数值越高，传输越稳定，延迟越高")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(8.0);
                    ui.add_enabled_ui(!locked, |ui| {
                        for value in [20, 60, 120, 240] {
                            ui.radio_value(
                                &mut self.settings.buffer_ms,
                                value,
                                format!("{value} ms"),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.settings.buffer_ms, 0, "自定义缓冲");
                            ui.add(
                                egui::DragValue::new(&mut self.settings.custom_buffer_ms)
                                    .range(20..=240)
                                    .suffix(" ms"),
                            );
                        });
                    });
                    ui.label(format!(
                        "当前使用：{} ms",
                        if self.settings.buffer_ms == 0 {
                            self.settings.custom_buffer_ms
                        } else {
                            self.settings.buffer_ms
                        }
                    ));
                });
                if locked && ui.button("请关闭传输后再调整设置").clicked() {
                    self.show_settings_lock = true;
                }
            }
        });
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn format_connection_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

impl AudioRouterApp {
    fn tick_backend(&mut self) {
        while let Ok((request_id, enabled, previous_enabled, result)) =
            self.startup_registration.result_rx.try_recv()
        {
            if request_id != self.startup_registration_request_id {
                continue;
            }
            if let Err(error) = result {
                if let Some(previous_enabled) = previous_enabled {
                    if self.settings.launch_on_startup == enabled {
                        self.settings.launch_on_startup = previous_enabled;
                        if let Err(save_error) = self.settings.save() {
                            self.last_action = save_error;
                        }
                    }
                }
                self.last_action = error;
            }
        }
        self.poll_capture();
        while let Ok(error) = self.playback_error_rx.try_recv() {
            self.last_action = error;
        }
        self.bluetooth_played_packets = self.playback_played_count.load(Ordering::Relaxed);
        self.bluetooth_playback_errors = self.playback_error_count.load(Ordering::Relaxed);
        if self.bluetooth_server.is_some()
            && self.bluetooth_scan_at.elapsed() >= Duration::from_secs(5)
        {
            self.bluetooth_scan_at = Instant::now();
            for paired in bluetooth::paired_devices() {
                let paired_key = format!("bt-paired:{:012X}", paired.address);
                let already_known = self.devices.values().any(|device| {
                    device.transport == TransportKind::Bluetooth
                        && (device.key == paired_key || device.name == paired.name)
                });
                if !already_known {
                    self.ensure_device(
                        paired_key.clone(),
                        TransportKind::Bluetooth,
                        paired.name,
                        "已配对设备",
                        false,
                    );
                    if let Some(device) = self.devices.get_mut(&paired_key) {
                        device.bluetooth_address = Some(paired.address);
                    }
                    self.schedule_auto_connect(&paired_key);
                }
            }
        }
        let lan_events = self
            .lan_server
            .as_ref()
            .map(LanServer::poll)
            .unwrap_or_default();
        for event in lan_events {
            match event {
                ServerEvent::Discovered { address, name } => {
                    let existing_key = self.devices.iter().find_map(|(key, device)| {
                        (device.transport == TransportKind::Lan
                            && device
                                .lan_address
                                .is_some_and(|known| known.ip() == address.ip()))
                        .then_some(key.clone())
                    });
                    if let Some(key) = existing_key {
                        if let Some(device) = self.devices.get_mut(&key) {
                            device.lan_address = Some(address);
                            if !device.connected {
                                device.name = Self::friendly_device_name(&name, Some(&device.name));
                            }
                        }
                    } else {
                        let key = format!("lan-discovered:{address}");
                        self.ensure_device(
                            key.clone(),
                            TransportKind::Lan,
                            name,
                            "局域网设备".to_string(),
                            false,
                        );
                        if let Some(device) = self.devices.get_mut(&key) {
                            device.lan_address = Some(address);
                        }
                        self.schedule_auto_connect(&key);
                    }
                }
                ServerEvent::Listening(address) => {
                    self.last_action = format!("局域网主机已监听：{address}")
                }
                ServerEvent::ClientConnected(address) => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    self.remove_lan_duplicates(address, Some(&key));
                    let discovered_key = format!("lan-discovered:{address}");
                    let discovered_key = self
                        .devices
                        .keys()
                        .find(|candidate| {
                            candidate.starts_with("lan-discovered:")
                                && self
                                    .devices
                                    .get(*candidate)
                                    .and_then(|device| device.lan_address)
                                    .is_some_and(|known| known.ip() == address.ip())
                        })
                        .cloned()
                        .unwrap_or(discovered_key);
                    self.promote_device(
                        &discovered_key,
                        key.clone(),
                        TransportKind::Lan,
                        "局域网连接",
                        false,
                    );
                    self.ensure_device(
                        key.clone(),
                        TransportKind::Lan,
                        "局域网设备",
                        "正在连接",
                        false,
                    );
                    if let Some(device) = self.devices.get_mut(&key) {
                        device.lan_address = Some(address);
                    }
                    self.last_action = "正在连接局域网设备，等待握手".to_string();
                }
                ServerEvent::ClientDisconnected(address) => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    self.lan_opus_encoders.remove(&address);
                    self.lan_opus_decoders
                        .retain(|(peer, _, _), _| *peer != address);
                    if let Some(device) = self.devices.get_mut(&key) {
                        device.connected = false;
                        device.connected_at = None;
                    }
                    self.schedule_auto_connect(&key);
                    self.fail_auto_connect(&key);
                    self.last_action = format!("设备已断开：{address}");
                }
                ServerEvent::PeerHello {
                    address,
                    device_id,
                    name,
                } => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    self.ensure_device(
                        key.clone(),
                        TransportKind::Lan,
                        name.clone(),
                        "局域网连接",
                        true,
                    );
                    self.restore_device_settings(&key);
                    self.sync_device_controls(&key);
                    self.remember_auto_target(&key);
                    let target = self.auto_target_id(&key);
                    self.auto_connect_tasks.remove(&target);
                    let _ = device_id;
                    self.last_action = format!("已连接局域网设备：{name}")
                }
                ServerEvent::Control { address, message } => {
                    self.last_action = format!("收到控制消息 {message:?} ({address})")
                }
                ServerEvent::Audio { address, packet } => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    let device = self.devices.get(&key);
                    if device.is_some_and(|device| device.connected && device.receive_enabled) {
                        let volume = device.map(|device| device.receive_volume).unwrap_or(100);
                        match self.decode_lan_packet(address, packet) {
                            Ok(packet) => self.enqueue_playback(packet, volume),
                            Err(error) => self.last_action = error,
                        }
                    }
                }
                ServerEvent::Error(message) => self.last_action = message,
            }
        }
        let bluetooth_events = self
            .bluetooth_server
            .as_ref()
            .map(|server| server.poll().take(64).collect::<Vec<_>>())
            .unwrap_or_default();
        for event in bluetooth_events {
            match event {
                BluetoothEvent::Listening => self.last_action = "蓝牙主机已启动".to_string(),
                BluetoothEvent::ClientConnected { key, address } => {
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    if address != 0 {
                        self.devices.retain(|old_key, device| {
                            !(old_key.starts_with("bt:")
                                && !device.connected
                                && device.bluetooth_address == Some(address))
                        });
                    }
                    let paired_key = format!("bt-paired:{address:012X}");
                    let paired_name = self
                        .devices
                        .get(&paired_key)
                        .map(|device| device.name.clone())
                        .or_else(|| {
                            bluetooth::paired_devices()
                                .into_iter()
                                .find(|device| device.address == address)
                                .map(|device| device.name)
                        })
                        .unwrap_or_else(|| "蓝牙设备".to_string());
                    self.promote_device(
                        &paired_key,
                        device_key.clone(),
                        TransportKind::Bluetooth,
                        "蓝牙连接",
                        false,
                    );
                    self.ensure_device(
                        device_key.clone(),
                        TransportKind::Bluetooth,
                        paired_name,
                        "正在连接",
                        false,
                    );
                    if let Some(device) = self.devices.get_mut(&device_key) {
                        device.bluetooth_address = (address != 0).then_some(address);
                    }
                    self.restore_device_settings(&device_key);
                    self.last_action = "正在连接蓝牙设备，等待握手".to_string();
                }
                BluetoothEvent::PeerHello {
                    key,
                    device_id,
                    name,
                } => {
                    self.devices.retain(|device_key, device| {
                        !(device.transport == TransportKind::Bluetooth
                            && !device.connected
                            && device_key.starts_with("bt-paired:")
                            && device.name == name)
                    });
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    self.ensure_device(
                        device_key.clone(),
                        TransportKind::Bluetooth,
                        name.clone(),
                        "蓝牙连接",
                        true,
                    );
                    self.restore_device_settings(&device_key);
                    self.sync_device_controls(&Self::device_key(TransportKind::Bluetooth, key));
                    self.remember_auto_target(&device_key);
                    let target = self.auto_target_id(&device_key);
                    self.auto_connect_tasks.remove(&target);
                    let _ = device_id;
                    self.last_action = format!("已连接蓝牙设备：{name}");
                }
                BluetoothEvent::ClientDisconnected { key } => {
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    self.bluetooth_opus_encoders.remove(&key);
                    if let Some(device) = self.devices.get_mut(&device_key) {
                        device.connected = false;
                        device.connected_at = None;
                    }
                    self.schedule_auto_connect(&device_key);
                    self.fail_auto_connect(&device_key);
                    self.last_action = "蓝牙设备已断开".to_string();
                }
                BluetoothEvent::Audio { key, packet } => {
                    self.bluetooth_received_packets += 1;
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    let device = self.devices.get(&device_key);
                    if device.is_some_and(|device| device.connected && device.receive_enabled) {
                        let volume = device.map(|device| device.receive_volume).unwrap_or(100);
                        match self.decode_bluetooth_packet(packet) {
                            Ok(packet) => {
                                self.bluetooth_decoded_packets += 1;
                                self.enqueue_playback(packet, volume);
                            }
                            Err(error) => {
                                self.bluetooth_playback_errors += 1;
                                self.last_action = error;
                            }
                        }
                    }
                }
                BluetoothEvent::Control { key, message } => {
                    self.last_action = format!("收到蓝牙控制消息 {message:?}（设备 {key}）");
                }
                BluetoothEvent::Error(message) => self.last_action = message,
            }
        }
        self.process_auto_connect_tasks();
        self.update_capture_state();
    }
}

impl eframe::App for AudioRouterApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_capture();
        while let Ok(error) = self.playback_error_rx.try_recv() {
            self.last_action = error;
        }
        self.bluetooth_played_packets = self.playback_played_count.load(Ordering::Relaxed);
        self.bluetooth_playback_errors = self.playback_error_count.load(Ordering::Relaxed);
        if self.bluetooth_server.is_some()
            && self.bluetooth_scan_at.elapsed() >= Duration::from_secs(5)
        {
            self.bluetooth_scan_at = Instant::now();
            for paired in bluetooth::paired_devices() {
                let paired_key = format!("bt-paired:{:012X}", paired.address);
                let already_known = self.devices.values().any(|device| {
                    device.transport == TransportKind::Bluetooth
                        && (device.key == paired_key || device.name == paired.name)
                });
                if !already_known {
                    self.ensure_device(
                        paired_key.clone(),
                        TransportKind::Bluetooth,
                        paired.name,
                        "已配对设备",
                        false,
                    );
                    if let Some(device) = self.devices.get_mut(&paired_key) {
                        device.bluetooth_address = Some(paired.address);
                    }
                }
            }
        }
        let lan_events = self
            .lan_server
            .as_ref()
            .map(LanServer::poll)
            .unwrap_or_default();
        for event in lan_events {
            match event {
                ServerEvent::Discovered { address, name } => {
                    let existing_key = self.devices.iter().find_map(|(key, device)| {
                        (device.transport == TransportKind::Lan
                            && device
                                .lan_address
                                .is_some_and(|known| known.ip() == address.ip()))
                        .then_some(key.clone())
                    });
                    if let Some(key) = existing_key {
                        if let Some(device) = self.devices.get_mut(&key) {
                            device.lan_address = Some(address);
                            if !device.connected {
                                device.name = Self::friendly_device_name(&name, Some(&device.name));
                            }
                        }
                    } else {
                        let key = format!("lan-discovered:{address}");
                        self.ensure_device(
                            key.clone(),
                            TransportKind::Lan,
                            name,
                            "局域网设备".to_string(),
                            false,
                        );
                        if let Some(device) = self.devices.get_mut(&key) {
                            device.lan_address = Some(address);
                        }
                    }
                }
                ServerEvent::Listening(address) => {
                    self.last_action = format!("局域网主机已监听：{address}")
                }
                ServerEvent::ClientConnected(address) => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    self.remove_lan_duplicates(address, Some(&key));
                    let discovered_key = format!("lan-discovered:{address}");
                    let discovered_key = self
                        .devices
                        .keys()
                        .find(|candidate| {
                            candidate.starts_with("lan-discovered:")
                                && self
                                    .devices
                                    .get(*candidate)
                                    .and_then(|device| device.lan_address)
                                    .is_some_and(|known| known.ip() == address.ip())
                        })
                        .cloned()
                        .unwrap_or(discovered_key);
                    self.promote_device(
                        &discovered_key,
                        key.clone(),
                        TransportKind::Lan,
                        "局域网连接",
                        false,
                    );
                    self.ensure_device(
                        key.clone(),
                        TransportKind::Lan,
                        "局域网设备",
                        "正在连接",
                        false,
                    );
                    if let Some(device) = self.devices.get_mut(&key) {
                        device.lan_address = Some(address);
                    }
                    self.last_action = "正在连接局域网设备，等待握手".to_string();
                }
                ServerEvent::ClientDisconnected(address) => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    self.lan_opus_encoders.remove(&address);
                    self.lan_opus_decoders
                        .retain(|(peer, _, _), _| *peer != address);
                    if let Some(device) = self.devices.get_mut(&key) {
                        device.connected = false;
                        device.connected_at = None;
                    }
                    self.last_action = format!("设备已断开：{address}");
                }
                ServerEvent::PeerHello {
                    address,
                    device_id,
                    name,
                } => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    self.ensure_device(
                        key.clone(),
                        TransportKind::Lan,
                        name.clone(),
                        "局域网连接",
                        true,
                    );
                    self.restore_device_settings(&key);
                    self.sync_device_controls(&key);
                    let _ = device_id;
                    self.last_action = format!("已连接局域网设备：{name}")
                }
                ServerEvent::Control { address, message } => {
                    self.last_action = format!("收到控制消息 {message:?} ({address})")
                }
                ServerEvent::Audio { address, packet } => {
                    let key = Self::device_key(TransportKind::Lan, address);
                    let device = self.devices.get(&key);
                    if device.is_some_and(|device| device.connected && device.receive_enabled) {
                        let volume = device.map(|device| device.receive_volume).unwrap_or(100);
                        match self.decode_lan_packet(address, packet) {
                            Ok(packet) => self.enqueue_playback(packet, volume),
                            Err(error) => self.last_action = error,
                        }
                    }
                }
                ServerEvent::Error(message) => self.last_action = message,
            }
        }
        let bluetooth_events = self
            .bluetooth_server
            .as_ref()
            .map(|server| server.poll().take(64).collect::<Vec<_>>())
            .unwrap_or_default();
        for event in bluetooth_events {
            match event {
                BluetoothEvent::Listening => self.last_action = "蓝牙主机已启动".to_string(),
                BluetoothEvent::ClientConnected { key, address } => {
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    if address != 0 {
                        self.devices.retain(|old_key, device| {
                            !(old_key.starts_with("bt:")
                                && !device.connected
                                && device.bluetooth_address == Some(address))
                        });
                    }
                    let paired_key = format!("bt-paired:{address:012X}");
                    let paired_name = self
                        .devices
                        .get(&paired_key)
                        .map(|device| device.name.clone())
                        .or_else(|| {
                            bluetooth::paired_devices()
                                .into_iter()
                                .find(|device| device.address == address)
                                .map(|device| device.name)
                        })
                        .unwrap_or_else(|| "蓝牙设备".to_string());
                    self.promote_device(
                        &paired_key,
                        device_key.clone(),
                        TransportKind::Bluetooth,
                        "蓝牙连接",
                        false,
                    );
                    self.ensure_device(
                        device_key.clone(),
                        TransportKind::Bluetooth,
                        paired_name,
                        "正在连接",
                        false,
                    );
                    if let Some(device) = self.devices.get_mut(&device_key) {
                        device.bluetooth_address = (address != 0).then_some(address);
                    }
                    self.restore_device_settings(&device_key);
                    self.last_action = "正在连接蓝牙设备，等待握手".to_string();
                }
                BluetoothEvent::PeerHello {
                    key,
                    device_id,
                    name,
                } => {
                    self.devices.retain(|device_key, device| {
                        !(device.transport == TransportKind::Bluetooth
                            && !device.connected
                            && device_key.starts_with("bt-paired:")
                            && device.name == name)
                    });
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    self.ensure_device(
                        device_key.clone(),
                        TransportKind::Bluetooth,
                        name.clone(),
                        "蓝牙连接",
                        true,
                    );
                    self.restore_device_settings(&device_key);
                    self.sync_device_controls(&Self::device_key(TransportKind::Bluetooth, key));
                    let _ = device_id;
                    self.last_action = format!("已连接蓝牙设备：{name}");
                }
                BluetoothEvent::ClientDisconnected { key } => {
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    self.bluetooth_opus_encoders.remove(&key);
                    if let Some(device) = self.devices.get_mut(&device_key) {
                        device.connected = false;
                        device.connected_at = None;
                    }
                    self.last_action = "蓝牙设备已断开".to_string();
                }
                BluetoothEvent::Audio { key, packet } => {
                    self.bluetooth_received_packets += 1;
                    let device_key = Self::device_key(TransportKind::Bluetooth, key);
                    let device = self.devices.get(&device_key);
                    if device.is_some_and(|device| device.connected && device.receive_enabled) {
                        let volume = device.map(|device| device.receive_volume).unwrap_or(100);
                        match self.decode_bluetooth_packet(packet) {
                            Ok(packet) => {
                                self.bluetooth_decoded_packets += 1;
                                self.enqueue_playback(packet, volume);
                            }
                            Err(error) => {
                                self.bluetooth_playback_errors += 1;
                                self.last_action = error;
                            }
                        }
                    }
                }
                BluetoothEvent::Control { key, message } => {
                    self.last_action = format!("收到蓝牙控制消息 {message:?}（设备 {key}）");
                }
                BluetoothEvent::Error(message) => self.last_action = message,
            }
        }
        self.update_capture_state();
        context.set_theme(match self.settings.theme {
            AppTheme::Light => egui::ThemePreference::Light,
            AppTheme::Dark => egui::ThemePreference::Dark,
            AppTheme::System => egui::ThemePreference::System,
        });

        let background = if context.style().visuals.dark_mode {
            egui::Color32::from_rgb(32, 39, 37)
        } else {
            egui::Color32::from_rgb(233, 239, 235)
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(background)
                    .inner_margin(egui::Margin::same(24.0)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("⌁")
                            .size(26.0)
                            .color(egui::Color32::from_rgb(143, 221, 195)),
                    );
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("AudioRouter").strong().size(16.0));
                        ui.label(
                            egui::RichText::new("实时音频连接")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new("⚙").size(21.0))
                            .on_hover_text("设置")
                            .clicked()
                        {
                            self.settings_open = !self.settings_open;
                            self.settings_category = None;
                        }
                        ui.add_space(12.0);
                        let mut capture_enabled = self.send_playback;
                        let capture_text = if capture_enabled {
                            "●  开"
                        } else {
                            "○  关"
                        };
                        let capture_response = ui.add(
                            egui::Button::new(egui::RichText::new(capture_text).size(12.0))
                                .selected(capture_enabled),
                        );
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("音频采集").small().strong());
                            ui.label(
                                egui::RichText::new(if capture_enabled {
                                    "采集中"
                                } else {
                                    "已停止"
                                })
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        });
                        if capture_response.clicked() {
                            capture_enabled = !capture_enabled;
                            self.capture_requested = capture_enabled;
                            if !capture_enabled {
                                let keys = self
                                    .devices
                                    .values()
                                    .filter(|device| device.connected && device.send_enabled)
                                    .map(|device| device.key.clone())
                                    .collect::<Vec<_>>();
                                for key in keys {
                                    if let Some(device) = self.devices.get_mut(&key) {
                                        device.send_enabled = false;
                                    }
                                    self.send_device_control(
                                        &key,
                                        ControlMessage::SetReceiveEnabled(false),
                                    );
                                }
                            }
                            self.update_capture_state();
                        }
                    });
                });
                ui.add_space(28.0);
                if self.settings_open {
                    self.render_settings(ui);
                } else {
                    let panel_height = ui.available_height().max(460.0);
                    ui.horizontal_top(|ui| {
                        let panel_width = ((ui.available_width() - 16.0) / 2.0).max(300.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(panel_width, panel_height),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| self.render_transport_panel(ui, TransportKind::Lan),
                        );
                        ui.add_space(16.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(panel_width, panel_height),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| self.render_transport_panel(ui, TransportKind::Bluetooth),
                        );
                    });
                }
            });
        if self.show_settings_lock {
            egui::Window::new("设置暂不可调整")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(context, |ui| {
                    ui.label("请关闭传输后再调整设置");
                    if ui.button("知道了").clicked() {
                        self.show_settings_lock = false;
                    }
                });
        }
        context.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    let _single_instance = match single_instance::Guard::acquire() {
        Some(guard) => guard,
        None => return Ok(()),
    };
    webview::run().map_err(|error| eframe::Error::AppCreation(Box::new(error)))
}
