use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{
    IpAddr, Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket,
};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use audiorouter_core::{
    capability_opus_bitrate_kbps, capability_preference, AudioPacket, Codec, ControlMessage,
    PacketKind, CAPABILITY_OPUS, HEADER_LEN, MAGIC, MAX_PAYLOAD_LEN, PREFERENCE_PCM,
    PROTOCOL_VERSION,
};

pub const LAN_PORT: u16 = 40400;
pub const DISCOVERY_PORT: u16 = 40401;
const LOCAL_CAPABILITIES: u32 = 0x07 | (128 << 10);
const MAX_CLIENT_QUEUE: usize = 3;
const TCP_AUDIO_SEND_WINDOW_MS: u64 = 40;
const MIN_TCP_AUDIO_SEND_BUFFER_BYTES: u32 = 1024;
const MAX_TCP_AUDIO_SEND_BUFFER_BYTES: u32 = 32 * 1024;
const RAW_AUDIO_BITRATE_BPS: u32 = 48_000 * 2 * 16;
const DEFAULT_TCP_AUDIO_SEND_BUFFER_BYTES: u32 = 7_680;

pub fn startup_error_message(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AddrInUse || error.raw_os_error() == Some(10048) {
        format!("无法开启局域网连接：端口 {LAN_PORT} 已被占用。请完全退出其它 AudioRouter 窗口和托盘中的旧版本，再开启局域网连接。如果仍然失败，请检查其它程序是否占用了该端口。")
    } else if error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(10013)
    {
        format!(
            "无法开启局域网连接：系统拒绝使用端口 {LAN_PORT}。请检查网络安全软件和系统端口限制。"
        )
    } else {
        format!("局域网连接启动失败：{error}")
    }
}

pub fn recommended_tcp_audio_send_buffer_bytes(compressed: bool, bitrate_kbps: u32) -> u32 {
    let bitrate_bps = if compressed {
        bitrate_kbps.clamp(32, 450) * 1_000
    } else {
        RAW_AUDIO_BITRATE_BPS
    };
    (u64::from(bitrate_bps) * TCP_AUDIO_SEND_WINDOW_MS / 8_000).clamp(
        u64::from(MIN_TCP_AUDIO_SEND_BUFFER_BYTES),
        u64::from(MAX_TCP_AUDIO_SEND_BUFFER_BYTES),
    ) as u32
}

#[cfg(windows)]
fn set_tcp_audio_send_buffer(stream: &TcpStream, size_bytes: u32) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, SOCKET, SOCKET_ERROR, SOL_SOCKET, SO_SNDBUF,
    };

    let size_bytes = size_bytes.min(i32::MAX as u32) as i32;
    let result = unsafe {
        setsockopt(
            stream.as_raw_socket() as SOCKET,
            SOL_SOCKET,
            SO_SNDBUF,
            (&size_bytes as *const i32).cast(),
            size_of::<i32>() as i32,
        )
    };
    if result == SOCKET_ERROR {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_tcp_audio_send_buffer(_stream: &TcpStream, _size_bytes: u32) -> io::Result<()> {
    Ok(())
}

#[derive(Clone)]
struct LatestFrameQueue {
    inner: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
    closed: Arc<AtomicBool>,
}

impl LatestFrameQueue {
    fn new() -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(VecDeque::with_capacity(MAX_CLIENT_QUEUE)),
                Condvar::new(),
            )),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn push_latest(&self, frame: Vec<u8>) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let (frames, ready) = &*self.inner;
        let Ok(mut frames) = frames.lock() else {
            return;
        };
        if frames.len() >= MAX_CLIENT_QUEUE {
            frames.pop_front();
        }
        frames.push_back(frame);
        ready.notify_one();
    }

    fn pop_next(&self) -> Option<Vec<u8>> {
        let (queue, ready) = &*self.inner;
        let mut frames = queue.lock().ok()?;
        loop {
            if let Some(frame) = frames.pop_front() {
                return Some(frame);
            }
            if self.closed.load(Ordering::Relaxed) {
                return None;
            }
            frames = ready.wait(frames).ok()?;
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.inner.1.notify_all();
    }
}

fn start_audio_writer(writer: Arc<Mutex<TcpStream>>, queue: LatestFrameQueue) {
    thread::spawn(move || {
        while let Some(frame) = queue.pop_next() {
            if write_frame(&writer, &frame).is_err() {
                queue.close();
                break;
            }
        }
    });
}

fn canonical_peer_address(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(address.ip(), LAN_PORT)
}

fn remove_peer_if_current(peers: &PeerWriters, address: SocketAddr, peer: &Arc<LanPeer>) -> bool {
    let Ok(mut active) = peers.lock() else {
        return false;
    };
    if active
        .get(&address)
        .is_some_and(|current| Arc::ptr_eq(current, peer))
    {
        active.remove(&address);
        true
    } else {
        false
    }
}

struct LanPeer {
    writer: Arc<Mutex<TcpStream>>,
    closer: TcpStream,
    audio_writer: LatestFrameQueue,
    audio_send_buffer_bytes: AtomicU32,
    accepts_audio: AtomicBool,
    sends_audio: AtomicBool,
    supports_opus: AtomicBool,
    preferred_codec: AtomicU32,
    opus_bitrate_kbps: AtomicU32,
    device_id: AtomicU64,
    name: Mutex<String>,
}

type PeerWriters = Arc<Mutex<HashMap<SocketAddr, Arc<LanPeer>>>>;

#[derive(Debug)]
pub enum ServerEvent {
    Listening(SocketAddr),
    Discovered {
        address: SocketAddr,
        name: String,
    },
    ClientConnected(SocketAddr),
    PeerHello {
        address: SocketAddr,
        device_id: u64,
        name: String,
    },
    Control {
        address: SocketAddr,
        message: ControlMessage,
    },
    Audio {
        address: SocketAddr,
        packet: AudioPacket,
    },
    ClientDisconnected(SocketAddr),
    Error(String),
}

pub struct LanServer {
    events: Receiver<ServerEvent>,
    sender: Sender<ServerEvent>,
    stop: Arc<AtomicBool>,
    peers: PeerWriters,
}

impl LanServer {
    pub fn start(bind_address: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(bind_address)?;
        listener.set_nonblocking(true)?;
        let actual_address = listener.local_addr()?;
        let (sender, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_peers = Arc::clone(&peers);
        let listener_sender = sender.clone();
        thread::spawn(move || {
            let _ = listener_sender.send(ServerEvent::Listening(actual_address));
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, remote_address)) => {
                        let address = canonical_peer_address(remote_address);
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_nodelay(true);
                        let _ =
                            set_tcp_audio_send_buffer(&stream, DEFAULT_TCP_AUDIO_SEND_BUFFER_BYTES);
                        let writer = match stream.try_clone() {
                            Ok(writer) => Arc::new(Mutex::new(writer)),
                            Err(error) => {
                                let _ = listener_sender.send(ServerEvent::Error(format!(
                                    "{address} 创建写入通道失败：{error}"
                                )));
                                continue;
                            }
                        };
                        let closer = match stream.try_clone() {
                            Ok(closer) => closer,
                            Err(error) => {
                                let _ = listener_sender.send(ServerEvent::Error(format!(
                                    "{address} 创建断开通道失败：{error}"
                                )));
                                continue;
                            }
                        };
                        if let Ok(writer) = writer.lock() {
                            let _ = writer.set_nodelay(true);
                        }
                        let peer = Arc::new(LanPeer {
                            writer,
                            closer,
                            audio_writer: LatestFrameQueue::new(),
                            audio_send_buffer_bytes: AtomicU32::new(
                                DEFAULT_TCP_AUDIO_SEND_BUFFER_BYTES,
                            ),
                            accepts_audio: AtomicBool::new(true),
                            sends_audio: AtomicBool::new(true),
                            supports_opus: AtomicBool::new(false),
                            preferred_codec: AtomicU32::new(0),
                            opus_bitrate_kbps: AtomicU32::new(128),
                            device_id: AtomicU64::new(0),
                            name: Mutex::new("局域网设备".to_string()),
                        });
                        if let Ok(mut peers) = thread_peers.lock() {
                            peers.insert(address, Arc::clone(&peer));
                        }
                        start_audio_writer(Arc::clone(&peer.writer), peer.audio_writer.clone());
                        let _ = listener_sender.send(ServerEvent::ClientConnected(address));
                        let client_sender = listener_sender.clone();
                        let client_peers = Arc::clone(&thread_peers);
                        thread::spawn(move || {
                            handle_client(stream, address, client_sender, peer, client_peers)
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(error) => {
                        let _ = listener_sender
                            .send(ServerEvent::Error(format!("局域网监听失败：{error}")));
                        break;
                    }
                }
            }
        });
        let discovery_stop = Arc::clone(&stop);
        thread::spawn(move || run_discovery_responder(discovery_stop));
        let scanner_stop = Arc::clone(&stop);
        let scanner_sender = sender.clone();
        thread::spawn(move || run_discovery_scanner(scanner_stop, scanner_sender));
        Ok(Self {
            events,
            sender,
            stop,
            peers,
        })
    }

    pub fn poll(&self) -> Vec<ServerEvent> {
        self.events.try_iter().collect()
    }

    pub fn connect(&self, address: SocketAddr) -> Result<(), String> {
        if self
            .peers
            .lock()
            .map_err(|_| "局域网设备列表已损坏".to_string())?
            .contains_key(&address)
        {
            return Ok(());
        }
        let address = canonical_peer_address(address);
        let sender = self.sender.clone();
        let peers = Arc::clone(&self.peers);
        let stop = Arc::clone(&self.stop);
        thread::spawn(move || {
            let result =
                TcpStream::connect_timeout(&address, Duration::from_secs(3)).and_then(|stream| {
                    stream.set_nodelay(true)?;
                    let _ = set_tcp_audio_send_buffer(&stream, DEFAULT_TCP_AUDIO_SEND_BUFFER_BYTES);
                    let writer = Arc::new(Mutex::new(stream.try_clone()?));
                    let closer = stream.try_clone()?;
                    let peer = Arc::new(LanPeer {
                        writer: Arc::clone(&writer),
                        closer,
                        audio_writer: LatestFrameQueue::new(),
                        audio_send_buffer_bytes: AtomicU32::new(
                            DEFAULT_TCP_AUDIO_SEND_BUFFER_BYTES,
                        ),
                        accepts_audio: AtomicBool::new(true),
                        sends_audio: AtomicBool::new(true),
                        supports_opus: AtomicBool::new(false),
                        preferred_codec: AtomicU32::new(0),
                        opus_bitrate_kbps: AtomicU32::new(128),
                        device_id: AtomicU64::new(0),
                        name: Mutex::new("局域网设备".to_string()),
                    });
                    if let Ok(mut peers) = peers.lock() {
                        peers.insert(address, Arc::clone(&peer));
                    }
                    let _ = sender.send(ServerEvent::ClientConnected(address));
                    let hello = ControlMessage::Hello {
                        device_id: 0,
                        device_name: "AudioRouter Windows".to_string(),
                        capabilities: LOCAL_CAPABILITIES,
                    }
                    .encode()
                    .map_err(|error| io::Error::other(error.to_string()))?;
                    write_control_kind(&writer, &hello, PacketKind::Hello as u8)?;
                    start_audio_writer(Arc::clone(&peer.writer), peer.audio_writer.clone());
                    handle_client(stream, address, sender.clone(), peer, peers);
                    Ok(())
                });
            if let Err(error) = result {
                let _ = sender.send(ServerEvent::Error(format!(
                    "连接局域网设备 {address} 失败：{error}"
                )));
            }
            let _ = stop;
        });
        Ok(())
    }

    pub fn send_control(&self, address: SocketAddr, message: ControlMessage) -> Result<(), String> {
        let peer = self
            .peers
            .lock()
            .map_err(|_| "局域网设备列表已损坏".to_string())?
            .get(&address)
            .cloned()
            .ok_or_else(|| "局域网设备未连接".to_string())?;
        apply_peer_control(&peer, &message);
        let payload = message
            .encode()
            .map_err(|error| format!("控制消息编码失败：{error}"))?;
        write_control_kind(&peer.writer, &payload, PacketKind::Hello as u8)
            .map_err(|error| format!("发送控制消息失败：{error}"))
    }

    pub fn disconnect(&self, address: SocketAddr) -> Result<(), String> {
        let peer = self
            .peers
            .lock()
            .map_err(|_| "局域网设备列表已损坏".to_string())?
            .remove(&address)
            .ok_or_else(|| "局域网设备未连接".to_string())?;
        peer.audio_writer.close();
        peer.closer
            .shutdown(Shutdown::Both)
            .map_err(|error| format!("断开局域网设备失败：{error}"))
    }

    pub fn peer_supports_opus(&self, address: SocketAddr) -> bool {
        self.peers
            .lock()
            .ok()
            .and_then(|peers| peers.get(&address).cloned())
            .is_some_and(|peer| peer.supports_opus.load(Ordering::Relaxed))
    }

    pub fn broadcast_pcm(
        &self,
        session_id: u64,
        sequence: u32,
        timestamp_ms: u64,
        sample_rate: u32,
        channels: u8,
        payload: &[u8],
    ) -> Result<usize, String> {
        let packet = AudioPacket::new_with_codec(
            session_id,
            sequence,
            timestamp_ms,
            sample_rate,
            channels,
            Codec::Pcm16Le,
            payload.to_vec(),
        )
        .map_err(|error| format!("创建发送音频包失败：{error}"))?;
        let frame = packet
            .encode()
            .map_err(|error| format!("编码发送音频包失败：{error}"))?;
        let peers = self
            .peers
            .lock()
            .map_err(|_| "局域网设备列表已损坏".to_string())?
            .iter()
            .filter(|(_, peer)| peer.accepts_audio.load(Ordering::Relaxed))
            .map(|(address, peer)| (*address, Arc::clone(peer)))
            .collect::<Vec<_>>();
        let mut delivered = 0;
        for (_address, peer) in peers {
            peer.audio_writer.push_latest(frame.clone());
            delivered += 1;
        }
        Ok(delivered)
    }

    pub fn send_audio_to(
        &self,
        address: SocketAddr,
        session_id: u64,
        sequence: u32,
        timestamp_ms: u64,
        sample_rate: u32,
        channels: u8,
        pcm_payload: &[u8],
        opus_frame: Option<&[u8]>,
        opus_bitrate_kbps: u32,
    ) -> Result<bool, String> {
        let peer = self
            .peers
            .lock()
            .map_err(|_| "局域网设备列表已损坏".to_string())?
            .get(&address)
            .cloned()
            .ok_or_else(|| "局域网设备未连接".to_string())?;
        if !peer.accepts_audio.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let use_opus = peer.supports_opus.load(Ordering::Relaxed)
            && peer.preferred_codec.load(Ordering::Relaxed) != u32::from(PREFERENCE_PCM)
            && opus_frame.is_some();
        let send_buffer_bytes =
            recommended_tcp_audio_send_buffer_bytes(use_opus, opus_bitrate_kbps);
        if peer.audio_send_buffer_bytes.load(Ordering::Relaxed) != send_buffer_bytes {
            if let Ok(stream) = peer.writer.lock() {
                let _ = set_tcp_audio_send_buffer(&stream, send_buffer_bytes);
                peer.audio_send_buffer_bytes
                    .store(send_buffer_bytes, Ordering::Relaxed);
            }
        }
        if use_opus {
            let frame = opus_frame.expect("checked Opus frame");
            peer.audio_writer.push_latest(frame.to_vec());
            return Ok(true);
        }
        let packet = AudioPacket::new_with_codec(
            session_id,
            sequence,
            timestamp_ms,
            sample_rate,
            channels,
            Codec::Pcm16Le,
            pcm_payload.to_vec(),
        )
        .map_err(|error| format!("创建发送音频包失败：{error}"))?;
        let frame = packet
            .encode()
            .map_err(|error| format!("编码发送音频包失败：{error}"))?;
        peer.audio_writer.push_latest(frame);
        Ok(true)
    }
}

fn run_discovery_responder(stop: Arc<AtomicBool>) {
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) else {
        return;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
    let device_name = local_device_name();
    let response = format!("AUDIOROUTER_DEVICE_V1|{LAN_PORT}|{device_name}");
    let mut buffer = [0u8; 128];
    while !stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buffer) {
            Ok((length, address)) if &buffer[..length] == b"AUDIOROUTER_DISCOVER_V1" => {
                let _ = socket.send_to(response.as_bytes(), address);
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
}

fn run_discovery_scanner(stop: Arc<AtomicBool>, sender: Sender<ServerEvent>) {
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)) else {
        return;
    };
    let _ = socket.set_broadcast(true);
    let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
    let request = b"AUDIOROUTER_DISCOVER_V1";
    let local_ip = local_ipv4();
    let local_name = local_device_name();
    let destinations = discovery_broadcasts();
    let mut buffer = [0u8; 256];
    while !stop.load(Ordering::Relaxed) {
        for destination in &destinations {
            let _ = socket.send_to(request, *destination);
        }
        let deadline = Instant::now() + Duration::from_millis(700);
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buffer) {
                Ok((length, source)) => {
                    let response = String::from_utf8_lossy(&buffer[..length]);
                    let mut fields = response.splitn(3, '|');
                    let (Some(prefix), Some(port), Some(name)) =
                        (fields.next(), fields.next(), fields.next())
                    else {
                        continue;
                    };
                    let Ok(port) = port.parse::<u16>() else {
                        continue;
                    };
                    if prefix == "AUDIOROUTER_DEVICE_V1"
                        && !name.trim().is_empty()
                        && Some(source.ip()) != local_ip.map(IpAddr::V4)
                        && name.trim() != local_name
                    {
                        let address = SocketAddr::new(source.ip(), port);
                        let _ = sender.send(ServerEvent::Discovered {
                            address,
                            name: name.trim().to_string(),
                        });
                    }
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn local_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .unwrap_or_else(|_| "AudioRouter Windows".to_string())
        .replace('|', " ")
}

fn local_ipv4() -> Option<Ipv4Addr> {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    probe.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    match probe.local_addr().ok()?.ip() {
        IpAddr::V4(address) if !address.is_unspecified() => Some(address),
        _ => None,
    }
}

fn discovery_broadcasts() -> Vec<SocketAddr> {
    let mut destinations = vec![SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::BROADCAST,
        DISCOVERY_PORT,
    ))];
    if let Some(address) = local_ipv4() {
        let octets = address.octets();
        let subnet_broadcast = Ipv4Addr::new(octets[0], octets[1], octets[2], 255);
        let destination = SocketAddr::V4(SocketAddrV4::new(subnet_broadcast, DISCOVERY_PORT));
        if !destinations.contains(&destination) {
            destinations.push(destination);
        }
    }
    destinations
}

impl Drop for LanServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut peers) = self.peers.lock() {
            peers.clear();
        }
    }
}

fn handle_client(
    mut stream: TcpStream,
    address: SocketAddr,
    sender: Sender<ServerEvent>,
    peer: Arc<LanPeer>,
    peers: PeerWriters,
) {
    loop {
        let mut header = [0u8; HEADER_LEN];
        if let Err(error) = stream.read_exact(&mut header) {
            if error.kind() != io::ErrorKind::UnexpectedEof {
                let _ = sender.send(ServerEvent::Error(format!("{address} 读取失败：{error}")));
            }
            if remove_peer_if_current(&peers, address, &peer) {
                let _ = sender.send(ServerEvent::ClientDisconnected(address));
            }
            break;
        }
        if header[0..4] != MAGIC || header[4] != PROTOCOL_VERSION {
            let _ = sender.send(ServerEvent::Error(format!(
                "{address} 协议版本或 Magic 无效"
            )));
            break;
        }
        let payload_len =
            u32::from_le_bytes(header[36..40].try_into().expect("header width")) as usize;
        if payload_len > MAX_PAYLOAD_LEN {
            let _ = sender.send(ServerEvent::Error(format!("{address} 负载过大")));
            break;
        }
        let mut frame = header.to_vec();
        frame.resize(HEADER_LEN + payload_len, 0);
        if let Err(error) = stream.read_exact(&mut frame[HEADER_LEN..]) {
            let _ = sender.send(ServerEvent::Error(format!(
                "{address} 读取负载失败：{error}"
            )));
            break;
        }
        match PacketKind::try_from(header[5]) {
            Ok(PacketKind::Audio) => match AudioPacket::decode(&frame) {
                Ok(packet) if peer.sends_audio.load(Ordering::Relaxed) => {
                    let _ = sender.send(ServerEvent::Audio { address, packet });
                }
                Ok(_) => {}
                Err(error) => {
                    let _ =
                        sender.send(ServerEvent::Error(format!("{address} 音频包无效：{error}")));
                    break;
                }
            },
            Ok(PacketKind::Hello) | Ok(PacketKind::HelloAck) => {
                let payload = &frame[HEADER_LEN..];
                match ControlMessage::decode(payload) {
                    Ok(message @ ControlMessage::Hello { .. })
                    | Ok(message @ ControlMessage::HelloAck { .. }) => {
                        let (device_id, device_name, _capabilities, reply) = match &message {
                            ControlMessage::Hello {
                                device_id,
                                device_name,
                                capabilities,
                            } => (*device_id, device_name.clone(), *capabilities, true),
                            ControlMessage::HelloAck {
                                device_id,
                                device_name,
                                capabilities,
                            } => (*device_id, device_name.clone(), *capabilities, false),
                            _ => unreachable!(),
                        };
                        peer.device_id.store(device_id, Ordering::Relaxed);
                        if let Ok(mut name) = peer.name.lock() {
                            *name = device_name.clone();
                        }
                        if reply {
                            let response = ControlMessage::HelloAck {
                                device_id,
                                device_name: "AudioRouter Windows".to_string(),
                                capabilities: LOCAL_CAPABILITIES,
                            };
                            match response.encode() {
                                Ok(payload) => {
                                    if let Err(error) = write_control(&peer.writer, &payload) {
                                        let _ = sender.send(ServerEvent::Error(format!(
                                            "{address} 握手回复失败：{error}"
                                        )));
                                        break;
                                    }
                                }
                                Err(error) => {
                                    let _ = sender.send(ServerEvent::Error(format!(
                                        "{address} 握手回复编码失败：{error}"
                                    )));
                                    break;
                                }
                            }
                        }
                        apply_peer_control(&peer, &message);
                        let _ = sender.send(ServerEvent::PeerHello {
                            address,
                            device_id,
                            name: device_name,
                        });
                    }
                    Ok(message) => {
                        apply_peer_control(&peer, &message);
                        let _ = sender.send(ServerEvent::Control { address, message });
                    }
                    Err(error) => {
                        let _ = sender
                            .send(ServerEvent::Error(format!("{address} 控制包无效：{error}")));
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                let _ = sender.send(ServerEvent::Error(format!("{address} 未知包类型：{error}")));
                break;
            }
        }
    }
    peer.audio_writer.close();
    if remove_peer_if_current(&peers, address, &peer) {
        let _ = sender.send(ServerEvent::ClientDisconnected(address));
    }
}

fn apply_peer_control(peer: &LanPeer, message: &ControlMessage) {
    match message {
        ControlMessage::Hello {
            device_id,
            device_name,
            capabilities,
        }
        | ControlMessage::HelloAck {
            device_id,
            device_name,
            capabilities,
        } => {
            peer.device_id.store(*device_id, Ordering::Relaxed);
            if let Ok(mut name) = peer.name.lock() {
                *name = device_name.clone();
            }
            peer.supports_opus
                .store(capabilities & CAPABILITY_OPUS != 0, Ordering::Relaxed);
            peer.preferred_codec.store(
                u32::from(capability_preference(*capabilities)),
                Ordering::Relaxed,
            );
            peer.opus_bitrate_kbps.store(
                capability_opus_bitrate_kbps(*capabilities).clamp(6, 450),
                Ordering::Relaxed,
            );
        }
        ControlMessage::SetSendEnabled(enabled) => {
            peer.sends_audio.store(*enabled, Ordering::Relaxed)
        }
        ControlMessage::SetReceiveEnabled(enabled) => {
            peer.accepts_audio.store(*enabled, Ordering::Relaxed)
        }
        _ => {}
    }
}

fn write_frame(writer: &Arc<Mutex<TcpStream>>, frame: &[u8]) -> io::Result<()> {
    let mut stream = writer
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "写入锁已损坏"))?;
    stream.write_all(frame)
}

fn write_control(writer: &Arc<Mutex<TcpStream>>, payload: &[u8]) -> io::Result<()> {
    write_control_kind(writer, payload, PacketKind::HelloAck as u8)
}

fn write_control_kind(
    writer: &Arc<Mutex<TcpStream>>,
    payload: &[u8],
    packet_kind: u8,
) -> io::Result<()> {
    write_frame(writer, &encode_control_kind(payload, packet_kind))
}

fn encode_control(payload: &[u8]) -> Vec<u8> {
    encode_control_kind(payload, PacketKind::HelloAck as u8)
}

fn encode_control_kind(payload: &[u8], packet_kind: u8) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&MAGIC);
    frame.push(PROTOCOL_VERSION);
    frame.push(packet_kind);
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&0u64.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&0u64.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&0u16.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn occupied_listener_reports_an_actionable_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let error = match LanServer::start(listener.local_addr().expect("listener address")) {
            Ok(_) => panic!("an occupied port should not start"),
            Err(error) => error,
        };
        let message = startup_error_message(&error);
        assert!(message.contains("已被占用"));
        assert!(message.contains("完全退出"));
    }

    #[test]
    fn startup_error_distinguishes_windows_port_restrictions() {
        assert!(startup_error_message(&io::Error::from_raw_os_error(10048)).contains("已被占用"));
        assert!(startup_error_message(&io::Error::from_raw_os_error(10013)).contains("系统拒绝"));
        assert!(startup_error_message(&io::Error::other("test failure")).contains("test failure"));
    }

    fn wait_for_listening(server: &LanServer) -> SocketAddr {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(ServerEvent::Listening(address)) = server.poll().into_iter().next() {
                return address;
            }
            assert!(Instant::now() < deadline, "server did not start");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn audio_writer_queue_drops_oldest_frame_and_stays_bounded() {
        let queue = LatestFrameQueue::new();
        for value in 1..=4 {
            queue.push_latest(vec![value]);
        }
        assert_eq!(queue.pop_next(), Some(vec![2]));
        assert_eq!(queue.pop_next(), Some(vec![3]));
        assert_eq!(queue.pop_next(), Some(vec![4]));
        queue.close();
        assert_eq!(queue.pop_next(), None);
    }

    #[test]
    fn tcp_audio_send_buffer_tracks_the_selected_bitrate() {
        assert_eq!(recommended_tcp_audio_send_buffer_bytes(false, 128), 7_680);
        assert_eq!(recommended_tcp_audio_send_buffer_bytes(true, 64), 1_024);
        assert_eq!(recommended_tcp_audio_send_buffer_bytes(true, 128), 1_024);
        assert_eq!(recommended_tcp_audio_send_buffer_bytes(true, 450), 2_250);
        assert_eq!(recommended_tcp_audio_send_buffer_bytes(true, 1), 1_024);
    }

    #[test]
    fn configures_tcp_audio_send_buffers_on_connected_sockets() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let client = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("connect client");
        let (server, _) = listener.accept().expect("accept client");

        set_tcp_audio_send_buffer(&client, 1_280).expect("configure client socket");
        set_tcp_audio_send_buffer(&server, 1_280).expect("configure server socket");
    }

    fn send_hello(stream: &mut TcpStream) {
        let payload = ControlMessage::Hello {
            device_id: 7,
            device_name: "AudioRouter Android 16".to_string(),
            capabilities: 3,
        }
        .encode()
        .expect("encode");
        stream
            .write_all(&encode_control(&payload))
            .expect("handshake");
    }

    #[test]
    fn accepts_android_style_handshake() {
        let server = LanServer::start("127.0.0.1:0".parse().expect("address")).expect("server");
        let mut stream = TcpStream::connect(wait_for_listening(&server)).expect("connect");
        send_hello(&mut stream);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_hello = false;
        while Instant::now() < deadline {
            for event in server.poll() {
                if let ServerEvent::PeerHello { name, .. } = event {
                    assert_eq!(name, "AudioRouter Android 16");
                    saw_hello = true;
                }
            }
            if saw_hello {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_hello, "server did not receive handshake");
    }

    #[test]
    fn broadcasts_pcm_to_handshaken_client() {
        let server = LanServer::start("127.0.0.1:0".parse().expect("address")).expect("server");
        let mut stream = TcpStream::connect(wait_for_listening(&server)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        send_hello(&mut stream);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            for event in server.poll() {
                if matches!(event, ServerEvent::PeerHello { .. }) {
                    let delivered = server
                        .broadcast_pcm(9, 1, 10, 48_000, 2, &[0, 1, 2, 3])
                        .expect("broadcast");
                    assert_eq!(delivered, 1);
                    let mut hello_header = [0u8; HEADER_LEN];
                    stream.read_exact(&mut hello_header).expect("hello header");
                    assert_eq!(hello_header[5], PacketKind::HelloAck as u8);
                    let hello_length =
                        u32::from_le_bytes(hello_header[36..40].try_into().unwrap()) as usize;
                    let mut hello_payload = vec![0; hello_length];
                    stream.read_exact(&mut hello_payload).expect("hello ack");
                    let mut audio_header = [0u8; HEADER_LEN];
                    stream.read_exact(&mut audio_header).expect("audio header");
                    assert_eq!(audio_header[5], PacketKind::Audio as u8);
                    return;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("server did not receive handshake");
    }

    #[test]
    fn per_peer_receive_switch_controls_broadcast() {
        let server = LanServer::start("127.0.0.1:0".parse().expect("address")).expect("server");
        let mut stream = TcpStream::connect(wait_for_listening(&server)).expect("connect");
        send_hello(&mut stream);
        let deadline = Instant::now() + Duration::from_secs(2);
        let address = loop {
            let mut found = None;
            for event in server.poll() {
                if let ServerEvent::PeerHello { address, .. } = event {
                    found = Some(address);
                }
            }
            if let Some(address) = found {
                break address;
            }
            assert!(
                Instant::now() < deadline,
                "server did not receive handshake"
            );
            thread::sleep(Duration::from_millis(10));
        };

        server
            .send_control(address, ControlMessage::SetReceiveEnabled(false))
            .expect("disable peer receive");
        assert_eq!(
            server
                .broadcast_pcm(1, 1, 1, 48_000, 2, &[0, 1])
                .expect("broadcast"),
            0
        );
        server
            .send_control(address, ControlMessage::SetReceiveEnabled(true))
            .expect("enable peer receive");
        assert_eq!(
            server
                .broadcast_pcm(1, 2, 2, 48_000, 2, &[0, 1])
                .expect("broadcast"),
            1
        );
    }
}
