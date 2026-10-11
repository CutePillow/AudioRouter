use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use audiorouter_core::{
    capability_preference, AudioPacket, ControlMessage, FrameDecoder, PacketKind, CAPABILITY_OPUS,
    PREFERENCE_PCM,
};

const MAX_CLIENT_QUEUE: usize = 3;
const LOCAL_CAPABILITIES: u32 = 0x07 | (128 << 10);

type PeerMap = Arc<Mutex<HashMap<usize, Arc<PeerConnection>>>>;

#[derive(Clone, Debug)]
pub struct PairedDevice {
    pub address: u64,
    pub name: String,
}

pub fn paired_devices() -> Vec<PairedDevice> {
    #[cfg(windows)]
    {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::Devices::Bluetooth::{
            BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice,
            BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS,
        };

        let mut search: BLUETOOTH_DEVICE_SEARCH_PARAMS = unsafe { zeroed() };
        search.dwSize = size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32;
        search.fReturnAuthenticated = 1;
        search.fReturnRemembered = 1;
        search.fReturnConnected = 1;
        let mut info: BLUETOOTH_DEVICE_INFO = unsafe { zeroed() };
        info.dwSize = size_of::<BLUETOOTH_DEVICE_INFO>() as u32;
        let handle = unsafe { BluetoothFindFirstDevice(&search, &mut info) };
        if handle.is_null() {
            return Vec::new();
        }

        let mut devices = Vec::new();
        loop {
            let name_end = info
                .szName
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(info.szName.len());
            let name = String::from_utf16_lossy(&info.szName[..name_end]);
            let address = unsafe { info.Address.Anonymous.ullLong };
            devices.push(PairedDevice {
                address,
                name: if name.is_empty() {
                    format!("蓝牙设备 {address:012X}")
                } else {
                    name
                },
            });
            if unsafe { BluetoothFindNextDevice(handle, &mut info) } == 0 {
                break;
            }
        }
        unsafe {
            BluetoothFindDeviceClose(handle);
        }
        devices
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[derive(Clone)]
struct LatestFrameQueue {
    inner: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
}

impl LatestFrameQueue {
    fn new() -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(VecDeque::with_capacity(MAX_CLIENT_QUEUE)),
                Condvar::new(),
            )),
        }
    }

    fn push_latest(&self, frame: Vec<u8>) {
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

    fn try_send(&self, frame: Vec<u8>) -> Result<(), ()> {
        self.push_latest(frame);
        Ok(())
    }

    fn pop_next(&self, stop: &AtomicBool) -> Option<Vec<u8>> {
        let (frames, ready) = &*self.inner;
        let mut frames = frames.lock().ok()?;
        loop {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(frame) = frames.pop_front() {
                return Some(frame);
            }
            let (next_frames, _) = ready.wait_timeout(frames, Duration::from_millis(50)).ok()?;
            frames = next_frames;
        }
    }
}

struct PeerConnection {
    socket: usize,
    writer: LatestFrameQueue,
    accepts_audio: AtomicBool,
    sends_audio: AtomicBool,
    supports_opus: AtomicBool,
    preferred_codec: AtomicU32,
    device_id: AtomicU64,
    name: Mutex<String>,
}

#[derive(Debug)]
pub enum ServerEvent {
    Listening,
    ClientConnected {
        key: usize,
        address: u64,
    },
    PeerHello {
        key: usize,
        device_id: u64,
        name: String,
    },
    ClientDisconnected {
        key: usize,
    },
    Audio {
        key: usize,
        packet: AudioPacket,
    },
    Control {
        key: usize,
        message: ControlMessage,
    },
    Error(String),
}

pub struct BluetoothServer {
    events: Receiver<ServerEvent>,
    sender: mpsc::Sender<ServerEvent>,
    peers: PeerMap,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl BluetoothServer {
    pub fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = start_platform_server(sender.clone(), Arc::clone(&peers), Arc::clone(&stop))?;
        Ok(Self {
            events: receiver,
            sender,
            peers,
            stop,
            worker: Some(worker),
        })
    }

    pub fn poll(&self) -> impl Iterator<Item = ServerEvent> + '_ {
        self.events.try_iter()
    }

    pub fn connect(&self, address: u64) -> Result<(), String> {
        #[cfg(windows)]
        {
            let sender = self.sender.clone();
            let peers = Arc::clone(&self.peers);
            let stop = Arc::clone(&self.stop);
            thread::spawn(move || {
                if let Err(error) = connect_platform_client(address, sender.clone(), peers, stop) {
                    let _ = sender.send(ServerEvent::Error(format!(
                        "连接蓝牙设备 {address:012X} 失败：{error}"
                    )));
                }
            });
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = address;
            Err("当前平台未实现 Bluetooth RFCOMM 客户端".to_string())
        }
    }

    pub fn send_control(&self, key: usize, message: ControlMessage) -> Result<(), String> {
        let peer = self
            .peers
            .lock()
            .map_err(|_| "蓝牙设备列表已损坏".to_string())?
            .get(&key)
            .cloned()
            .ok_or_else(|| "蓝牙设备未连接".to_string())?;
        apply_peer_control(&peer, &message);
        let payload = message
            .encode()
            .map_err(|error| format!("控制消息编码失败：{error}"))?;
        peer.writer
            .try_send(encode_control_frame(PacketKind::Hello as u8, &payload))
            .map_err(|_| "蓝牙控制消息队列不可用".to_string())
    }

    pub fn disconnect(&self, key: usize) -> Result<(), String> {
        let socket = self
            .peers
            .lock()
            .map_err(|_| "蓝牙设备列表已损坏".to_string())?
            .get(&key)
            .map(|peer| peer.socket)
            .ok_or_else(|| "蓝牙设备未连接".to_string())?;
        #[cfg(windows)]
        {
            use windows_sys::Win32::Networking::WinSock::{
                shutdown, WSAGetLastError, SD_BOTH, SOCKET_ERROR,
            };
            if unsafe { shutdown(socket, SD_BOTH) } == SOCKET_ERROR {
                return Err(format!("断开蓝牙设备失败：{}", unsafe {
                    WSAGetLastError()
                }));
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = socket;
            Err("当前平台未实现 Bluetooth RFCOMM 断开连接".to_string())
        }
    }

    pub fn peer_uses_opus(&self, key: usize) -> bool {
        self.peers
            .lock()
            .ok()
            .and_then(|peers| peers.get(&key).cloned())
            .is_some_and(|peer| {
                peer.supports_opus.load(Ordering::Relaxed)
                    && peer.preferred_codec.load(Ordering::Relaxed) != u32::from(PREFERENCE_PCM)
            })
    }

    pub fn peer_supports_opus(&self, key: usize) -> bool {
        self.peers
            .lock()
            .ok()
            .and_then(|peers| peers.get(&key).cloned())
            .is_some_and(|peer| peer.supports_opus.load(Ordering::Relaxed))
    }

    pub fn send_audio_frame_to(
        &self,
        key: usize,
        pcm_frame: &[u8],
        opus_frame: Option<&[u8]>,
    ) -> Result<bool, String> {
        let peer = self
            .peers
            .lock()
            .map_err(|_| "蓝牙设备列表已损坏".to_string())?
            .get(&key)
            .cloned()
            .ok_or_else(|| "蓝牙设备未连接".to_string())?;
        if !peer.accepts_audio.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let frame = if peer.supports_opus.load(Ordering::Relaxed)
            && peer.preferred_codec.load(Ordering::Relaxed) != u32::from(PREFERENCE_PCM)
        {
            opus_frame.unwrap_or(pcm_frame)
        } else {
            pcm_frame
        };
        peer.writer.push_latest(frame.to_vec());
        Ok(true)
    }
}

impl Drop for BluetoothServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(windows)]
        {
            use windows_sys::Win32::Networking::WinSock::{shutdown, SD_BOTH};
            if let Ok(peers) = self.peers.lock() {
                for peer in peers.values() {
                    unsafe { shutdown(peer.socket, SD_BOTH) };
                }
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn start_platform_server(
    sender: mpsc::Sender<ServerEvent>,
    peers: PeerMap,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    use std::mem::{size_of, zeroed};
    use std::ptr::null_mut;

    use windows_sys::core::GUID;
    use windows_sys::Win32::Devices::Bluetooth::{
        BluetoothFindFirstRadio, BluetoothFindRadioClose, AF_BTH, BLUETOOTH_FIND_RADIO_PARAMS,
        BTHPROTO_RFCOMM, SOCKADDR_BTH,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Networking::WinSock::{
        accept, bind, closesocket, getsockname, ioctlsocket, listen, socket, WSAGetLastError,
        WSAStartup, AFPROTOCOLS, FIONBIO, INVALID_SOCKET, RNRSERVICE_DELETE, RNRSERVICE_REGISTER,
        SOCKADDR, SOCKET_ERROR, SOCK_STREAM, SOMAXCONN, WSADATA, WSAEWOULDBLOCK,
    };

    let radio_params = BLUETOOTH_FIND_RADIO_PARAMS {
        dwSize: size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
    };
    let mut radio: HANDLE = null_mut();
    let radio_finder = unsafe { BluetoothFindFirstRadio(&radio_params, &mut radio) };
    if radio_finder.is_null() {
        return Err("未检测到 Windows 蓝牙适配器".to_string());
    }
    unsafe {
        BluetoothFindRadioClose(radio_finder);
        if !radio.is_null() {
            CloseHandle(radio);
        }
    }

    let mut wsa_data: WSADATA = unsafe { zeroed() };
    let startup_result = unsafe { WSAStartup(0x0202, &mut wsa_data) };
    if startup_result != 0 {
        return Err(format!("初始化 Winsock 失败：{startup_result}"));
    }

    let listener = unsafe { socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32) };
    if listener == INVALID_SOCKET {
        let error = unsafe { WSAGetLastError() };
        unsafe { windows_sys::Win32::Networking::WinSock::WSACleanup() };
        return Err(format!("创建 Bluetooth RFCOMM 套接字失败：{error}"));
    }

    let mut service_class_id = GUID {
        data1: 0xb5c9f1b4,
        data2: 0x7a9c,
        data3: 0x4c0e,
        data4: [0x9e, 0x3c, 0x4e, 0x4f, 0x9b, 0x2f, 0x7d, 0x01],
    };
    let mut address = SOCKADDR_BTH {
        addressFamily: AF_BTH,
        btAddr: 0,
        serviceClassId: service_class_id,
        port: u32::MAX,
    };
    let mut nonblocking = 1u32;
    if unsafe { ioctlsocket(listener, FIONBIO, &mut nonblocking) } == SOCKET_ERROR {
        let error = unsafe { WSAGetLastError() };
        unsafe {
            closesocket(listener);
            windows_sys::Win32::Networking::WinSock::WSACleanup();
        }
        return Err(format!("设置 Bluetooth RFCOMM 套接字失败：{error}"));
    }

    if unsafe {
        bind(
            listener,
            &address as *const SOCKADDR_BTH as *const SOCKADDR,
            size_of::<SOCKADDR_BTH>() as i32,
        )
    } == SOCKET_ERROR
    {
        let error = unsafe { WSAGetLastError() };
        unsafe {
            closesocket(listener);
            windows_sys::Win32::Networking::WinSock::WSACleanup();
        }
        return Err(format!("绑定 Bluetooth 服务失败：{error}"));
    }
    if unsafe { listen(listener, SOMAXCONN as i32) } == SOCKET_ERROR {
        let error = unsafe { WSAGetLastError() };
        unsafe {
            closesocket(listener);
            windows_sys::Win32::Networking::WinSock::WSACleanup();
        }
        return Err(format!("启动 Bluetooth 服务失败：{error}"));
    }

    let mut address_length = size_of::<SOCKADDR_BTH>() as i32;
    if unsafe {
        getsockname(
            listener,
            &mut address as *mut SOCKADDR_BTH as *mut SOCKADDR,
            &mut address_length,
        )
    } == SOCKET_ERROR
    {
        let error = unsafe { WSAGetLastError() };
        unsafe {
            closesocket(listener);
            windows_sys::Win32::Networking::WinSock::WSACleanup();
        }
        return Err(format!("读取 Bluetooth RFCOMM 通道失败：{error}"));
    }

    let mut service_name: Vec<u16> = "AudioRouter\0".encode_utf16().collect();
    let mut protocol = AFPROTOCOLS {
        iAddressFamily: AF_BTH as i32,
        iProtocol: BTHPROTO_RFCOMM as i32,
    };
    publish_service(
        &mut address,
        &mut service_name,
        &mut service_class_id,
        &mut protocol,
        RNRSERVICE_REGISTER,
    )
    .inspect_err(|_| unsafe {
        closesocket(listener);
        windows_sys::Win32::Networking::WinSock::WSACleanup();
    })?;

    let worker = thread::spawn(move || {
        let _ = sender.send(ServerEvent::Listening);
        let mut client_workers = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut remote_address: SOCKADDR_BTH = unsafe { zeroed() };
            let mut remote_address_length = size_of::<SOCKADDR_BTH>() as i32;
            let client = unsafe {
                accept(
                    listener,
                    &mut remote_address as *mut SOCKADDR_BTH as *mut SOCKADDR,
                    &mut remote_address_length,
                )
            };
            if client == INVALID_SOCKET {
                let error = unsafe { WSAGetLastError() };
                if error == WSAEWOULDBLOCK {
                    thread::sleep(Duration::from_millis(40));
                    continue;
                }
                if !stop.load(Ordering::Relaxed) {
                    let _ = sender.send(ServerEvent::Error(format!(
                        "接受 Bluetooth 客户端失败：{error}"
                    )));
                }
                break;
            }
            let mut client_nonblocking = 1u32;
            if unsafe { ioctlsocket(client, FIONBIO, &mut client_nonblocking) } == SOCKET_ERROR {
                unsafe { closesocket(client) };
                continue;
            }
            let key = client as usize;
            let writer = LatestFrameQueue::new();
            let peer = Arc::new(PeerConnection {
                socket: key,
                writer,
                accepts_audio: AtomicBool::new(true),
                sends_audio: AtomicBool::new(true),
                supports_opus: AtomicBool::new(false),
                preferred_codec: AtomicU32::new(0),
                device_id: AtomicU64::new(0),
                name: Mutex::new("蓝牙设备".to_string()),
            });
            if let Ok(mut active) = peers.lock() {
                active.insert(key, Arc::clone(&peer));
            }
            let _ = sender.send(ServerEvent::ClientConnected {
                key,
                address: remote_address.btAddr,
            });

            let writer_peer = Arc::clone(&peer);
            let writer_stop = Arc::clone(&stop);
            client_workers.push(thread::spawn(move || {
                bluetooth_write_loop(client, writer_peer, writer_stop)
            }));
            let reader_sender = sender.clone();
            let reader_peers = Arc::clone(&peers);
            let reader_stop = Arc::clone(&stop);
            client_workers.push(thread::spawn(move || {
                read_client(client, peer, reader_sender, reader_peers, reader_stop)
            }));
        }

        stop.store(true, Ordering::Relaxed);
        if let Ok(active) = peers.lock() {
            for peer in active.values() {
                unsafe {
                    windows_sys::Win32::Networking::WinSock::shutdown(
                        peer.socket,
                        windows_sys::Win32::Networking::WinSock::SD_BOTH,
                    )
                };
            }
        }
        for worker in client_workers {
            let _ = worker.join();
        }
        let _ = publish_service(
            &mut address,
            &mut service_name,
            &mut service_class_id,
            &mut protocol,
            RNRSERVICE_DELETE,
        );
        unsafe {
            closesocket(listener);
            windows_sys::Win32::Networking::WinSock::WSACleanup();
        }
    });
    Ok(worker)
}

#[cfg(windows)]
fn publish_service(
    address: &mut windows_sys::Win32::Devices::Bluetooth::SOCKADDR_BTH,
    service_name: &mut [u16],
    service_class_id: &mut windows_sys::core::GUID,
    protocol: &mut windows_sys::Win32::Networking::WinSock::AFPROTOCOLS,
    operation: i32,
) -> Result<(), String> {
    use std::mem::size_of;
    use std::ptr::null_mut;
    use windows_sys::Win32::Devices::Bluetooth::{BTHPROTO_RFCOMM, NS_BTH, SOCKADDR_BTH};
    use windows_sys::Win32::Networking::WinSock::{
        WSAGetLastError, WSASetServiceW, CSADDR_INFO, SOCKET_ADDRESS, SOCKET_ERROR, SOCK_STREAM,
        WSAQUERYSETW,
    };

    let mut socket_address = CSADDR_INFO {
        LocalAddr: SOCKET_ADDRESS {
            lpSockaddr: address as *mut SOCKADDR_BTH as *mut _,
            iSockaddrLength: size_of::<SOCKADDR_BTH>() as i32,
        },
        RemoteAddr: SOCKET_ADDRESS {
            lpSockaddr: null_mut(),
            iSockaddrLength: 0,
        },
        iSocketType: SOCK_STREAM,
        iProtocol: BTHPROTO_RFCOMM as i32,
    };
    let query = WSAQUERYSETW {
        dwSize: size_of::<WSAQUERYSETW>() as u32,
        lpszServiceInstanceName: service_name.as_mut_ptr(),
        lpServiceClassId: service_class_id,
        lpVersion: null_mut(),
        lpszComment: null_mut(),
        dwNameSpace: NS_BTH,
        lpNSProviderId: null_mut(),
        lpszContext: null_mut(),
        dwNumberOfProtocols: 1,
        lpafpProtocols: protocol,
        lpszQueryString: null_mut(),
        dwNumberOfCsAddrs: 1,
        lpcsaBuffer: &mut socket_address,
        dwOutputFlags: 0,
        lpBlob: null_mut(),
    };
    if unsafe { WSASetServiceW(&query, operation, 0) } == SOCKET_ERROR {
        return Err(format!("发布 Bluetooth SDP 服务失败：{}", unsafe {
            WSAGetLastError()
        }));
    }
    Ok(())
}

#[cfg(windows)]
fn connect_platform_client(
    address: u64,
    sender: mpsc::Sender<ServerEvent>,
    peers: PeerMap,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    use std::mem::size_of;
    use windows_sys::Win32::Devices::Bluetooth::{AF_BTH, BTHPROTO_RFCOMM, SOCKADDR_BTH};
    use windows_sys::Win32::Networking::WinSock::{
        closesocket, connect, ioctlsocket, socket, WSAGetLastError, FIONBIO, INVALID_SOCKET,
        SOCKADDR, SOCKET_ERROR, SOCK_STREAM,
    };

    let client = unsafe { socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32) };
    if client == INVALID_SOCKET {
        return Err(format!("创建 Bluetooth RFCOMM 套接字失败：{}", unsafe {
            WSAGetLastError()
        }));
    }
    let service_class_id = windows_sys::core::GUID {
        data1: 0xb5c9f1b4,
        data2: 0x7a9c,
        data3: 0x4c0e,
        data4: [0x9e, 0x3c, 0x4e, 0x4f, 0x9b, 0x2f, 0x7d, 0x01],
    };
    let remote = SOCKADDR_BTH {
        addressFamily: AF_BTH,
        btAddr: address,
        serviceClassId: service_class_id,
        port: 0,
    };
    let connected = unsafe {
        connect(
            client,
            &remote as *const SOCKADDR_BTH as *const SOCKADDR,
            size_of::<SOCKADDR_BTH>() as i32,
        )
    };
    if connected == SOCKET_ERROR {
        let error = unsafe { WSAGetLastError() };
        unsafe { closesocket(client) };
        return Err(format!("Bluetooth RFCOMM 连接失败：{error}"));
    }
    let mut nonblocking = 1u32;
    if unsafe { ioctlsocket(client, FIONBIO, &mut nonblocking) } == SOCKET_ERROR {
        let error = unsafe { WSAGetLastError() };
        unsafe { closesocket(client) };
        return Err(format!("设置 Bluetooth 客户端套接字失败：{error}"));
    }

    let key = client as usize;
    let peer = Arc::new(PeerConnection {
        socket: key,
        writer: LatestFrameQueue::new(),
        accepts_audio: AtomicBool::new(true),
        sends_audio: AtomicBool::new(true),
        supports_opus: AtomicBool::new(false),
        preferred_codec: AtomicU32::new(0),
        device_id: AtomicU64::new(0),
        name: Mutex::new("蓝牙设备".to_string()),
    });
    if let Ok(mut active) = peers.lock() {
        active.insert(key, Arc::clone(&peer));
    }
    let _ = sender.send(ServerEvent::ClientConnected { key, address });
    let hello = ControlMessage::Hello {
        device_id: 0,
        device_name: "AudioRouter Windows".to_string(),
        capabilities: LOCAL_CAPABILITIES,
    }
    .encode()
    .map_err(|error| error.to_string())?;
    peer.writer
        .try_send(encode_control_frame(PacketKind::Hello as u8, &hello))
        .map_err(|_| "Bluetooth 握手消息队列不可用".to_string())?;

    let writer_peer = Arc::clone(&peer);
    let writer_stop = Arc::clone(&stop);
    thread::spawn(move || bluetooth_write_loop(client, writer_peer, writer_stop));
    let reader_sender = sender.clone();
    let reader_peers = Arc::clone(&peers);
    let reader_stop = Arc::clone(&stop);
    thread::spawn(move || read_client(client, peer, reader_sender, reader_peers, reader_stop));
    Ok(())
}

#[cfg(windows)]
fn bluetooth_write_loop(
    client: windows_sys::Win32::Networking::WinSock::SOCKET,
    peer: Arc<PeerConnection>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        let Some(frame) = peer.writer.pop_next(&stop) else {
            break;
        };
        if !send_all(client, &frame, &stop) {
            break;
        }
    }
    let _ = peer;
}

#[cfg(windows)]
fn send_all(
    client: windows_sys::Win32::Networking::WinSock::SOCKET,
    frame: &[u8],
    stop: &AtomicBool,
) -> bool {
    use windows_sys::Win32::Networking::WinSock::{
        send, WSAGetLastError, SOCKET_ERROR, WSAEWOULDBLOCK,
    };
    let mut offset = 0;
    while offset < frame.len() && !stop.load(Ordering::Relaxed) {
        let sent = unsafe {
            send(
                client,
                frame[offset..].as_ptr(),
                (frame.len() - offset).min(i32::MAX as usize) as i32,
                0,
            )
        };
        if sent > 0 {
            offset += sent as usize;
        } else if sent == SOCKET_ERROR && unsafe { WSAGetLastError() } == WSAEWOULDBLOCK {
            thread::sleep(Duration::from_millis(5));
        } else {
            return false;
        }
    }
    offset == frame.len()
}

#[cfg(windows)]
fn read_client(
    client: windows_sys::Win32::Networking::WinSock::SOCKET,
    peer: Arc<PeerConnection>,
    sender: mpsc::Sender<ServerEvent>,
    peers: PeerMap,
    stop: Arc<AtomicBool>,
) {
    use windows_sys::Win32::Networking::WinSock::{
        closesocket, recv, WSAGetLastError, SOCKET_ERROR, WSAEWOULDBLOCK,
    };

    let mut decoder = FrameDecoder::default();
    let mut buffer = [0u8; 8192];
    while !stop.load(Ordering::Relaxed) {
        let received = unsafe { recv(client, buffer.as_mut_ptr(), buffer.len() as i32, 0) };
        if received == 0 {
            break;
        }
        if received == SOCKET_ERROR {
            if unsafe { WSAGetLastError() } == WSAEWOULDBLOCK {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            break;
        }
        match decoder.push(&buffer[..received as usize]) {
            Ok(frames) => {
                for frame in frames {
                    match PacketKind::try_from(frame[5]) {
                        Ok(PacketKind::Audio) => match AudioPacket::decode(&frame) {
                            Ok(packet) if peer.sends_audio.load(Ordering::Relaxed) => {
                                let _ = sender.send(ServerEvent::Audio {
                                    key: peer.socket,
                                    packet,
                                });
                            }
                            Ok(_) => {}
                            Err(error) => {
                                let _ = sender
                                    .send(ServerEvent::Error(format!("蓝牙音频包错误：{error}")));
                            }
                        },
                        Ok(PacketKind::Hello) | Ok(PacketKind::HelloAck) => {
                            match ControlMessage::decode(&frame[40..]) {
                                Ok(message) => {
                                    if let ControlMessage::Hello { device_id, .. } = &message {
                                        let ack = ControlMessage::HelloAck {
                                            device_id: *device_id,
                                            device_name: "AudioRouter Windows".to_string(),
                                            capabilities: LOCAL_CAPABILITIES,
                                        };
                                        if let Ok(payload) = ack.encode() {
                                            let _ = peer
                                                .writer
                                                .try_send(encode_control_frame(3, &payload));
                                        }
                                    }
                                    apply_peer_control(&peer, &message);
                                    if let ControlMessage::Hello {
                                        device_id,
                                        device_name,
                                        ..
                                    } = &message
                                    {
                                        peer.device_id.store(*device_id, Ordering::Relaxed);
                                        if let Ok(mut name) = peer.name.lock() {
                                            *name = device_name.clone();
                                        }
                                        let _ = sender.send(ServerEvent::PeerHello {
                                            key: peer.socket,
                                            device_id: *device_id,
                                            name: device_name.clone(),
                                        });
                                    }
                                    let _ = sender.send(ServerEvent::Control {
                                        key: peer.socket,
                                        message,
                                    });
                                }
                                Err(error) => {
                                    let _ = sender.send(ServerEvent::Error(format!(
                                        "蓝牙控制包错误：{error}"
                                    )));
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let _ =
                                sender.send(ServerEvent::Error(format!("蓝牙包类型错误：{error}")));
                        }
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(ServerEvent::Error(format!("蓝牙协议帧错误：{error}")));
                break;
            }
        }
    }

    if let Ok(mut active) = peers.lock() {
        active.remove(&peer.socket);
    }
    unsafe { closesocket(client) };
    let _ = sender.send(ServerEvent::ClientDisconnected { key: peer.socket });
}

#[cfg(windows)]
fn apply_peer_control(peer: &PeerConnection, message: &ControlMessage) {
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

#[cfg(windows)]
fn encode_control_frame(packet_kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(40 + payload.len());
    frame.extend_from_slice(b"ARAU");
    frame.extend_from_slice(&[1, packet_kind, 0, 0]);
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

#[cfg(not(windows))]
fn start_platform_server(
    _sender: mpsc::Sender<ServerEvent>,
    _peers: PeerMap,
    _stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    Err("当前平台未实现 Bluetooth RFCOMM 服务".to_string())
}

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_ack_frame_matches_shared_control_protocol() {
        let expected = ControlMessage::HelloAck {
            device_id: 42,
            device_name: "AudioRouter Windows".to_string(),
            capabilities: 7,
        };
        let payload = expected.encode().expect("encode hello ack");
        let frame = encode_control_frame(PacketKind::HelloAck as u8, &payload);
        let mut decoder = FrameDecoder::default();
        let frames = decoder.push(&frame).expect("decode frame");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0][5], PacketKind::HelloAck as u8);
        assert_eq!(
            ControlMessage::decode(&frames[0][40..]).expect("decode payload"),
            expected
        );
    }
}
