package com.audiorouter.app;

import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioTrack;
import android.os.Build;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

final class LanSession {
    interface Listener {
        void onConnected(String deviceName);
        void onDisconnected();
    }

    static final int DEFAULT_PORT = 40400;
    private static final int HEADER_SIZE = 40;
    private static final int MAX_PAYLOAD_SIZE = 64 * 1024;
    private static final byte[] MAGIC = new byte[]{'A', 'R', 'A', 'U'};
    private static final byte PROTOCOL_VERSION = 1;
    private static final int PACKET_AUDIO = 1;
    private static final int PACKET_CONTROL = 2;
    private static final int PACKET_HELLO_ACK = 3;
    private static final int CODEC_OPUS = 1;
    private static final int CODEC_PCM16LE = 2;

    private final AtomicBoolean running = new AtomicBoolean();
    private final Object writeLock = new Object();
    private final Listener listener;
    private volatile Socket socket;
    private volatile OutputStream output;
    private volatile ServerSocket serverSocket;
    private volatile DatagramSocket discoverySocket;
    private volatile boolean handshakeComplete;
    private volatile long connectedAtElapsedRealtime;
    private volatile String peerName = "局域网设备";
    private volatile String codecMode = AudioQuality.LAN_COMPRESSED;
    private volatile int opusBitrateKbps = AudioQuality.DEFAULT_LAN_OPUS_BITRATE_KBPS;
    private volatile int audioBufferMs = AudioRouterService.DEFAULT_AUDIO_BUFFER_MS;
    private final Object playbackLock = new Object();
    private final java.util.ArrayDeque<PlaybackFrame> playbackQueue = new java.util.ArrayDeque<>();
    private int playbackQueuedMs;
    private boolean playbackPrimed;
    private boolean playbackResetRequested;
    private final Thread playbackThread;
    private volatile boolean peerSupportsOpus;
    private AudioTrack audioTrack;
    private float receiveVolume = 1.0f;
    private volatile boolean remoteSendEnabled = true;
    private volatile boolean receiveEnabled = true;
    private long sessionId = System.nanoTime();
    private int sequence;
    private OpusAudioCodec.Encoder opusEncoder;
    private int opusEncoderChannels;
    private int opusEncoderBitrateKbps;
    private OpusAudioCodec.Decoder opusDecoder;
    private long opusDecoderSessionId;
    private int opusDecoderChannels;

    LanSession(Listener listener) {
        this.listener = listener;
        playbackThread = new Thread(this::playbackLoop, "AudioRouter-LAN-playback");
        playbackThread.setDaemon(true);
        playbackThread.start();
    }

    void connect(String host, int port) throws IOException {
        closeConnection();
        Socket newSocket = new Socket();
        newSocket.connect(new InetSocketAddress(host, port), 5000);
        newSocket.setTcpNoDelay(true);
        try {
            attachSocket(newSocket);
        } catch (IOException error) {
            try { newSocket.close(); } catch (IOException ignored) { }
            closeConnection(newSocket);
            throw error;
        }
    }

    void listen() throws IOException {
        if (serverSocket == null) {
            ServerSocket listener = new ServerSocket(DEFAULT_PORT);
            serverSocket = listener;
            Thread acceptThread = new Thread(() -> acceptLoop(listener), "AudioRouter-LAN-server");
            acceptThread.setDaemon(true);
            acceptThread.start();
        }
        if (discoverySocket == null) {
            DatagramSocket responder = new DatagramSocket(DiscoverySession.DISCOVERY_PORT);
            responder.setBroadcast(true);
            discoverySocket = responder;
            Thread discoveryThread = new Thread(() -> discoveryLoop(responder),
                    "AudioRouter-LAN-discovery");
            discoveryThread.setDaemon(true);
            discoveryThread.start();
        }
    }

    void setAudioQuality(String mode, int bitrateKbps) throws IOException {
        codecMode = AudioQuality.LAN_COMPRESSED.equals(mode)
                ? AudioQuality.LAN_COMPRESSED : AudioQuality.LAN_UNCOMPRESSED;
        opusBitrateKbps = Math.max(6, Math.min(450, bitrateKbps));
        opusEncoder = null;
        if (isConnected()) {
            sendControl(PACKET_CONTROL, buildHelloPayload());
        }
    }

    void setAudioBufferMs(int bufferMs) {
        audioBufferMs = Math.max(20, Math.min(240, bufferMs));
        synchronized (playbackLock) {
            playbackQueue.clear();
            playbackQueuedMs = 0;
            playbackPrimed = false;
            playbackLock.notifyAll();
        }
    }

    void sendAudio(byte[] pcm, int sampleRate, int channels) throws IOException {
        if (!running.get()) {
            return;
        }
        if (AudioQuality.LAN_COMPRESSED.equals(codecMode)
                && peerSupportsOpus
                && sampleRate == OpusAudioCodec.SAMPLE_RATE
                && (channels == 1 || channels == 2)) {
            try {
                if (opusEncoder == null || opusEncoderChannels != channels
                        || opusEncoderBitrateKbps != opusBitrateKbps) {
                    opusEncoder = new OpusAudioCodec.Encoder(channels, opusBitrateKbps);
                    opusEncoderChannels = channels;
                    opusEncoderBitrateKbps = opusBitrateKbps;
                }
                List<byte[]> packets = opusEncoder.encode(pcm);
                for (byte[] packet : packets) {
                    writeFrame(PACKET_AUDIO, sessionId, sequence++, System.currentTimeMillis(),
                            sampleRate, channels, CODEC_OPUS, packet);
                }
                return;
            } catch (IOException error) {
                opusEncoder = null;
            }
        }
        writeFrame(PACKET_AUDIO, sessionId, sequence++, System.currentTimeMillis(),
                sampleRate, channels, CODEC_PCM16LE, pcm);
    }

    void setSendEnabled(boolean enabled) throws IOException {
        sendControl(2, new byte[]{1, 3, (byte) (enabled ? 1 : 0)});
    }

    void setReceiveEnabled(boolean enabled) throws IOException {
        receiveEnabled = enabled;
        sendControl(2, new byte[]{1, 4, (byte) (enabled ? 1 : 0)});
    }

    void setSendVolume(int volume) throws IOException {
        sendControl(2, new byte[]{1, 5, (byte) Math.max(0, Math.min(100, volume))});
    }

    void setReceiveVolume(int volume) throws IOException {
        receiveVolume = PcmAudio.volumeToGain(volume);
        sendControl(2, new byte[]{1, 6, (byte) Math.max(0, Math.min(100, volume))});
    }

    boolean isConnected() {
        return running.get() && handshakeComplete;
    }

    long connectedAtElapsedRealtime() {
        return connectedAtElapsedRealtime;
    }

    String peerName() {
        return peerName;
    }

    String peerIdentity() {
        Socket current = socket;
        if (current == null || current.getInetAddress() == null) return "";
        int remotePort = current.getPort();
        if (current.getLocalPort() == DEFAULT_PORT && remotePort != DEFAULT_PORT) {
            remotePort = DEFAULT_PORT;
        }
        return current.getInetAddress().getHostAddress() + ":" + remotePort;
    }

    void awaitHandshake(long timeoutMillis) throws IOException {
        long deadline = System.currentTimeMillis() + timeoutMillis;
        while (running.get() && !handshakeComplete && System.currentTimeMillis() < deadline) {
            try {
                Thread.sleep(25);
            } catch (InterruptedException error) {
                Thread.currentThread().interrupt();
                closeConnection();
                throw new IOException("局域网握手被中断", error);
            }
        }
        if (!handshakeComplete) {
            closeConnection();
            throw new IOException("局域网握手超时");
        }
    }

    boolean isRemoteSendEnabled() {
        return remoteSendEnabled;
    }

    void close() {
        stopListening();
    }

    void disconnect() {
        closeConnection();
    }

    void stopListening() {
        closeConnection();
        ServerSocket listener = serverSocket;
        serverSocket = null;
        if (listener != null) {
            try { listener.close(); } catch (IOException ignored) { }
        }
        DatagramSocket responder = discoverySocket;
        discoverySocket = null;
        if (responder != null) responder.close();
    }

    private void acceptLoop(ServerSocket listener) {
        while (serverSocket == listener) {
            try {
                Socket accepted = listener.accept();
                if (serverSocket != listener) {
                    try { accepted.close(); } catch (IOException ignored) { }
                    return;
                }
                accepted.setTcpNoDelay(true);
                closeConnection();
                attachSocket(accepted);
            } catch (SocketException error) {
                if (serverSocket == listener) {
                    serverSocket = null;
                }
                return;
            } catch (IOException error) {
                if (serverSocket == listener) {
                    serverSocket = null;
                }
                return;
            }
        }
    }

    private void discoveryLoop(DatagramSocket responder) {
        byte[] requestBuffer = new byte[128];
        byte[] response = ("AUDIOROUTER_DEVICE_V1|" + DEFAULT_PORT + "|" + deviceName())
                .getBytes(StandardCharsets.UTF_8);
        while (discoverySocket == responder) {
            try {
                DatagramPacket request = new DatagramPacket(requestBuffer, requestBuffer.length);
                responder.receive(request);
                String value = new String(request.getData(), request.getOffset(), request.getLength(),
                        StandardCharsets.UTF_8);
                if ("AUDIOROUTER_DISCOVER_V1".equals(value)) {
                    DatagramPacket reply = new DatagramPacket(
                            response, response.length, request.getAddress(), request.getPort());
                    responder.send(reply);
                }
            } catch (IOException error) {
                if (discoverySocket == responder) discoverySocket = null;
                return;
            }
        }
    }

    private void attachSocket(Socket newSocket) throws IOException {
        socket = newSocket;
        output = newSocket.getOutputStream();
        running.set(true);
        handshakeComplete = false;
        connectedAtElapsedRealtime = 0;
        peerName = "局域网设备";
        peerSupportsOpus = false;
        resetCodecState();
        sendControl(PACKET_CONTROL, buildHelloPayload());
        Thread reader = new Thread(() -> readLoop(newSocket), "AudioRouter-LAN-reader");
        reader.setDaemon(true);
        reader.start();
    }

    private void closeConnection() {
        closeConnection(null);
    }

    private void closeConnection(Socket expected) {
        if (expected != null && socket != expected) {
            return;
        }
        boolean wasConnected = handshakeComplete;
        running.set(false);
        handshakeComplete = false;
        connectedAtElapsedRealtime = 0;
        remoteSendEnabled = true;
        peerSupportsOpus = false;
        resetCodecState();
        synchronized (playbackLock) {
            playbackQueue.clear();
            playbackQueuedMs = 0;
            playbackPrimed = false;
            playbackResetRequested = true;
            playbackLock.notifyAll();
        }
        Socket current = socket;
        socket = null;
        output = null;
        if (current != null) {
            try { current.close(); } catch (IOException ignored) { }
        }
        if (wasConnected) {
            listener.onDisconnected();
        }
    }

    private static String deviceName() {
        String model = Build.MODEL;
        if (model == null || model.trim().isEmpty()) return "Android 设备";
        return model.replace('|', ' ').trim();
    }

    private void readLoop(Socket current) {
        try {
            InputStream input = current.getInputStream();
            byte[] header = new byte[HEADER_SIZE];
            while (running.get()) {
                readFully(input, header, 0, HEADER_SIZE);
                validateHeader(header);
                int packetKind = header[5] & 0xff;
                int sampleRate = readInt(header, 28);
                int channels = header[32] & 0xff;
                int codec = header[33] & 0xff;
                int payloadLength = readInt(header, 36);
                if (payloadLength < 0 || payloadLength > MAX_PAYLOAD_SIZE) {
                    throw new IOException("音频负载过大");
                }
                byte[] payload = new byte[payloadLength];
                readFully(input, payload, 0, payloadLength);
                if (packetKind == PACKET_AUDIO && codec == CODEC_PCM16LE) {
                    playPcm(payload, sampleRate, channels);
                } else if (packetKind == PACKET_AUDIO && codec == CODEC_OPUS
                        && sampleRate == OpusAudioCodec.SAMPLE_RATE) {
                    byte[] pcm = decodeOpus(readLong(header, 8), payload, channels);
                    if (pcm != null) {
                        playPcm(pcm, sampleRate, channels);
                    }
                } else if (packetKind == PACKET_CONTROL || packetKind == PACKET_HELLO_ACK) {
                    respondToControl(payload);
                }
            }
        } catch (IOException error) {
            closeConnection(current);
        }
    }

    private void respondToControl(byte[] payload) throws IOException {
        if (payload.length < 2 || payload[0] != 1) {
            return;
        }
        if (payload[1] == 1) {
            updatePeerName(payload);
            updatePeerCapabilities(payload);
            sendControl(PACKET_HELLO_ACK, buildHelloPayloadForAck());
            completeHandshake();
            return;
        }
        if (payload[1] == 2) {
            updatePeerName(payload);
            updatePeerCapabilities(payload);
            completeHandshake();
            return;
        }
        if (payload.length != 3) {
            return;
        }
        int value = payload[2] & 0xff;
        switch (payload[1]) {
            case 3:
                remoteSendEnabled = value != 0;
                break;
            case 4:
                receiveEnabled = value != 0;
                break;
            case 5:
                break;
            case 6:
                receiveVolume = PcmAudio.volumeToGain(value);
                break;
            default:
                break;
        }
    }

    private void updatePeerName(byte[] payload) {
        if (payload.length < 12) return;
        int nameLength = ByteBuffer.wrap(payload, 10, 2)
                .order(ByteOrder.LITTLE_ENDIAN).getShort() & 0xffff;
        if (nameLength == 0 || 12 + nameLength > payload.length) return;
        String value = new String(payload, 12, nameLength, StandardCharsets.UTF_8).trim();
        if (!value.isEmpty()) peerName = value;
    }

    private void updatePeerCapabilities(byte[] payload) {
        if (payload.length < 16) return;
        int nameLength = ByteBuffer.wrap(payload, 10, 2)
                .order(ByteOrder.LITTLE_ENDIAN).getShort() & 0xffff;
        if (nameLength == 0 || payload.length != 16 + nameLength) return;
        peerSupportsOpus = AudioQuality.supportsOpus(readInt(payload, 12 + nameLength));
    }

    private void completeHandshake() {
        if (handshakeComplete) return;
        handshakeComplete = true;
        connectedAtElapsedRealtime = android.os.SystemClock.elapsedRealtime();
        listener.onConnected(peerName);
    }

    private byte[] decodeOpus(long sourceSessionId, byte[] packet, int channels)
            throws IOException {
        if (channels != 1 && channels != 2) return null;
        if (opusDecoder == null || opusDecoderSessionId != sourceSessionId
                || opusDecoderChannels != channels) {
            opusDecoder = new OpusAudioCodec.Decoder(channels);
            opusDecoderSessionId = sourceSessionId;
            opusDecoderChannels = channels;
        }
        return opusDecoder.decode(packet);
    }

    private void resetCodecState() {
        opusEncoder = null;
        opusDecoder = null;
        opusDecoderSessionId = 0;
        opusDecoderChannels = 0;
    }

    private void playPcm(byte[] pcm, int sampleRate, int channels) {
        if (!receiveEnabled) {
            return;
        }
        if (channels < 1 || channels > 2 || sampleRate <= 0) {
            return;
        }
        byte[] output = PcmAudio.applyPlaybackGain(pcm, receiveVolume);
        int frameMs = Math.max(1, output.length * 1000
                / (sampleRate * Math.max(1, channels) * 2));
        synchronized (playbackLock) {
            playbackQueue.addLast(new PlaybackFrame(output, sampleRate, channels, frameMs));
            playbackQueuedMs += frameMs;
            while (playbackQueuedMs > audioBufferMs + frameMs && playbackQueue.size() > 1) {
                playbackQueuedMs -= playbackQueue.removeFirst().durationMs;
            }
            if (playbackQueuedMs >= audioBufferMs) {
                playbackPrimed = true;
            }
            playbackLock.notifyAll();
        }
    }

    private void playbackLoop() {
        while (true) {
            PlaybackFrame frame = null;
            boolean resetTrack = false;
            synchronized (playbackLock) {
                while (!playbackResetRequested
                        && (!playbackPrimed || playbackQueue.isEmpty())) {
                    try {
                        playbackLock.wait();
                    } catch (InterruptedException error) {
                        Thread.currentThread().interrupt();
                        return;
                    }
                }
                if (playbackResetRequested) {
                    playbackResetRequested = false;
                    playbackPrimed = false;
                    resetTrack = true;
                } else {
                    frame = playbackQueue.removeFirst();
                    playbackQueuedMs -= frame.durationMs;
                    if (playbackQueue.isEmpty()) {
                        playbackPrimed = false;
                    }
                }
            }
            if (resetTrack) {
                closeAudioTrack();
            } else if (frame != null) {
                writePcm(frame.pcm, frame.sampleRate, frame.channels);
            }
        }
    }

    private void writePcm(byte[] output, int sampleRate, int channels) {
        int channelMask = channels == 1 ? AudioFormat.CHANNEL_OUT_MONO : AudioFormat.CHANNEL_OUT_STEREO;
        if (audioTrack == null) {
            int minBuffer = AudioTrack.getMinBufferSize(
                    sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT);
            if (minBuffer <= 0) {
                return;
            }
            audioTrack = new AudioTrack.Builder()
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                            .build())
                    .setAudioFormat(new AudioFormat.Builder()
                            .setSampleRate(sampleRate)
                            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                            .setChannelMask(channelMask)
                            .build())
                    .setBufferSizeInBytes(Math.max(minBuffer, output.length))
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .build();
            audioTrack.setVolume(1.0f);
            audioTrack.play();
        }
        audioTrack.write(output, 0, output.length, AudioTrack.WRITE_BLOCKING);
    }

    private static final class PlaybackFrame {
        private final byte[] pcm;
        private final int sampleRate;
        private final int channels;
        private final int durationMs;

        private PlaybackFrame(byte[] pcm, int sampleRate, int channels, int durationMs) {
            this.pcm = pcm;
            this.sampleRate = sampleRate;
            this.channels = channels;
            this.durationMs = durationMs;
        }
    }

    private byte[] buildHelloPayload() {
        return buildHelloPayloadWithType(1);
    }

    private byte[] buildHelloPayloadForAck() {
        return buildHelloPayloadWithType(2);
    }

    private byte[] buildHelloPayloadWithType(int type) {
        byte[] name = deviceName().getBytes(StandardCharsets.UTF_8);
        ByteBuffer payload = ByteBuffer.allocate(2 + 8 + 2 + name.length + 4)
                .order(ByteOrder.LITTLE_ENDIAN);
        payload.put((byte) 1).put((byte) type).putLong(sessionId)
                .putShort((short) name.length).put(name)
                .putInt(AudioQuality.buildLanCapabilities(codecMode, opusBitrateKbps));
        return payload.array();
    }

    private void sendControl(int packetKind, byte[] payload) throws IOException {
        writeFrame(packetKind, 0, 0, 0, 0, 0, 0, payload);
    }

    private void writeFrame(int packetKind, long frameSessionId, int frameSequence,
                            long timestamp, int sampleRate, int channels, int codec,
                            byte[] payload) throws IOException {
        if (payload.length > MAX_PAYLOAD_SIZE) {
            throw new IOException("负载过大");
        }
        ByteBuffer frame = ByteBuffer.allocate(HEADER_SIZE + payload.length)
                .order(ByteOrder.LITTLE_ENDIAN);
        frame.put(MAGIC).put(PROTOCOL_VERSION).put((byte) packetKind).put((byte) 0).put((byte) 0)
                .putLong(frameSessionId).putInt(frameSequence).putLong(timestamp)
                .putInt(sampleRate).put((byte) channels).put((byte) codec).putShort((short) 0)
                .putInt(payload.length).put(payload);
        synchronized (writeLock) {
            if (output == null) {
                throw new IOException("局域网连接未建立");
            }
            output.write(frame.array());
            output.flush();
        }
    }

    private static void validateHeader(byte[] header) throws IOException {
        for (int index = 0; index < MAGIC.length; index++) {
            if (header[index] != MAGIC[index]) {
                throw new IOException("无效的 AudioRouter 数据帧");
            }
        }
        if (header[4] != PROTOCOL_VERSION) {
            throw new IOException("协议版本不兼容");
        }
    }

    private static int readInt(byte[] bytes, int offset) {
        return ByteBuffer.wrap(bytes, offset, 4).order(ByteOrder.LITTLE_ENDIAN).getInt();
    }

    private static long readLong(byte[] bytes, int offset) {
        return ByteBuffer.wrap(bytes, offset, 8).order(ByteOrder.LITTLE_ENDIAN).getLong();
    }

    private static void readFully(InputStream input, byte[] buffer, int offset, int length)
            throws IOException {
        int read = 0;
        while (read < length) {
            int count = input.read(buffer, offset + read, length - read);
            if (count < 0) {
                throw new IOException("局域网连接已关闭");
            }
            read += count;
        }
    }

    private void closeAudioTrack() {
        if (audioTrack != null) {
            try {
                audioTrack.stop();
            } catch (IllegalStateException ignored) {
            }
            audioTrack.release();
            audioTrack = null;
        }
    }
}
