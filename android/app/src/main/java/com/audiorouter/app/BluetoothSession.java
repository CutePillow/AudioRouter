package com.audiorouter.app;

import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothServerSocket;
import android.bluetooth.BluetoothSocket;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioTrack;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class BluetoothSession {
    public static final UUID SERVICE_UUID = UUID.fromString("b5c9f1b4-7a9c-4c0e-9e3c-4e4f9b2f7d01");
    private static final int HEADER_SIZE = 40;
    private static final int MAX_PAYLOAD_SIZE = 64 * 1024;
    private static final byte[] MAGIC = new byte[]{'A', 'R', 'A', 'U'};
    private static final byte PROTOCOL_VERSION = 1;
    private static final int PACKET_AUDIO = 1;
    private static final int PACKET_CONTROL = 2;
    private static final int PACKET_HELLO_ACK = 3;
    private static final int CODEC_OPUS = 1;
    private static final int CODEC_PCM16LE = 2;

    public interface Listener {
        void onConnecting(BluetoothDevice device);
        void onConnected(BluetoothDevice device);
        void onDisconnected(String message);
        void onError(String message);
    }

    private final ExecutorService executor = Executors.newSingleThreadExecutor();
    private final ExecutorService audioEncoderExecutor = Executors.newSingleThreadExecutor();
    private final ExecutorService audioSenderExecutor = Executors.newSingleThreadExecutor();
    private final Listener listener;
    private final Object writeLock = new Object();
    private final Object codecLock = new Object();
    private final BlockingQueue<CapturedAudio> audioQueue = new ArrayBlockingQueue<>(3);
    private final BlockingQueue<OutgoingAudio> packetQueue = new ArrayBlockingQueue<>(3);
    private final Map<Long, DecoderState> opusDecoders = new HashMap<>();
    private volatile BluetoothSocket socket;
    private volatile BluetoothServerSocket serverSocket;
    private volatile boolean listenRequested;
    private volatile boolean running;
    private volatile boolean handshakeComplete;
    private volatile boolean opusNegotiated;
    private volatile String codecMode = AudioQuality.BLUETOOTH_AUTO;
    private volatile boolean remoteSendEnabled = true;
    private volatile boolean sendEnabled;
    private volatile boolean receiveEnabled = true;
    private volatile float receiveVolume = 1.0f;
    private volatile int audioBufferMs = AudioRouterService.DEFAULT_AUDIO_BUFFER_MS;
    private final java.util.ArrayDeque<byte[]> playbackQueue = new java.util.ArrayDeque<>();
    private int playbackQueuedMs;
    private volatile int sendVolumePercent = 100;
    private volatile int receiveVolumePercent = 100;
    private AudioTrack audioTrack;
    private int audioTrackSampleRate;
    private int audioTrackChannels;
    private long sessionId = System.nanoTime();
    private int sequence;
    private OpusAudioCodec.Encoder opusEncoder;

    public BluetoothSession(Listener listener) {
        this.listener = listener;
        audioEncoderExecutor.execute(this::audioEncodeLoop);
        audioSenderExecutor.execute(this::audioWriteLoop);
    }

    public void connect(BluetoothDevice device, boolean initialSendEnabled) {
        close();
        setInitialSendEnabled(initialSendEnabled);
        executor.execute(() -> connectInternal(device));
    }

    public void setInitialSendEnabled(boolean enabled) {
        sendEnabled = enabled;
        if (!enabled) audioQueue.clear();
    }

    public void setAudioQuality(String mode) throws IOException {
        if (AudioQuality.BLUETOOTH_OPUS.equals(mode)) {
            codecMode = AudioQuality.BLUETOOTH_OPUS;
        } else if (AudioQuality.BLUETOOTH_PCM.equals(mode)) {
            codecMode = AudioQuality.BLUETOOTH_PCM;
        } else {
            codecMode = AudioQuality.BLUETOOTH_AUTO;
        }
        synchronized (codecLock) {
            opusEncoder = null;
        }
        if (handshakeComplete) {
            sendControl(buildHelloPayload());
        }
    }

    public void setAudioBufferMs(int bufferMs) {
        audioBufferMs = Math.max(20, Math.min(240, bufferMs));
        synchronized (playbackQueue) {
            playbackQueue.clear();
            playbackQueuedMs = 0;
        }
    }

    public synchronized void listen() {
        if (listenRequested || serverSocket != null) return;
        listenRequested = true;
        Thread listenerThread = new Thread(this::listenInternal, "AudioRouter-Bluetooth-server");
        listenerThread.setDaemon(true);
        listenerThread.start();
    }

    public synchronized void stopListening() {
        listenRequested = false;
        BluetoothServerSocket listener = serverSocket;
        serverSocket = null;
        if (listener != null) {
            try { listener.close(); } catch (IOException ignored) { }
        }
    }

    public boolean isConnected() {
        BluetoothSocket current = socket;
        return running && handshakeComplete && current != null && current.isConnected();
    }

    public boolean isRemoteSendEnabled() {
        return remoteSendEnabled;
    }

    public void sendAudio(byte[] pcm, int sampleRate, int channels) throws IOException {
        if (!isConnected()) return;
        enqueueAudio(new CapturedAudio(sampleRate, channels, pcm.clone()));
    }

    public void setSendEnabled(boolean enabled) throws IOException {
        sendEnabled = enabled;
        if (!enabled) audioQueue.clear();
        sendControlIfReady(new byte[]{1, 3, (byte) (enabled ? 1 : 0)});
    }

    public void setReceiveEnabled(boolean enabled) throws IOException {
        receiveEnabled = enabled;
        sendControlIfReady(new byte[]{1, 4, (byte) (enabled ? 1 : 0)});
    }

    public void setSendVolume(int volume) throws IOException {
        sendVolumePercent = clamp(volume);
        sendControlIfReady(new byte[]{1, 5, (byte) sendVolumePercent});
    }

    public void setReceiveVolume(int volume) throws IOException {
        receiveVolumePercent = clamp(volume);
        receiveVolume = PcmAudio.volumeToGain(receiveVolumePercent);
        sendControlIfReady(new byte[]{1, 6, (byte) receiveVolumePercent});
    }

    public void close() {
        running = false;
        handshakeComplete = false;
        opusNegotiated = false;
        remoteSendEnabled = true;
        audioQueue.clear();
        packetQueue.clear();
        synchronized (codecLock) {
            opusEncoder = null;
            opusDecoders.clear();
        }
        closeAudioTrack();
        BluetoothSocket current = socket;
        socket = null;
        if (current != null) {
            try { current.close(); } catch (IOException ignored) { }
        }
    }

    public void disconnect() {
        close();
    }

    public void shutdown() {
        stopListening();
        close();
        executor.shutdownNow();
        audioEncoderExecutor.shutdownNow();
        audioSenderExecutor.shutdownNow();
    }

    private void enqueueAudio(CapturedAudio audio) {
        if (audioQueue.offer(audio)) {
            return;
        }
        audioQueue.poll();
        audioQueue.offer(audio);
    }

    private void audioEncodeLoop() {
        try {
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO);
        } catch (RuntimeException ignored) {
        }
        while (!Thread.currentThread().isInterrupted()) {
            try {
                CapturedAudio audio = audioQueue.take();
                if (!isConnected()) {
                    continue;
                }
                encodeCapturedAudio(audio);
            } catch (InterruptedException error) {
                Thread.currentThread().interrupt();
                return;
            } catch (IOException error) {
                listener.onError("蓝牙音频发送失败：" + error.getMessage());
                audioQueue.clear();
            }
        }
    }

    private void encodeCapturedAudio(CapturedAudio audio) throws IOException {
        if (shouldSendOpus() && audio.sampleRate == OpusAudioCodec.SAMPLE_RATE
                && (audio.channels == 1 || audio.channels == 2)) {
            synchronized (codecLock) {
                try {
                    if (opusEncoder == null || opusEncoder.channels() != audio.channels) {
                        opusEncoder = new OpusAudioCodec.Encoder(audio.channels);
                    }
                    List<byte[]> packets = opusEncoder.encode(audio.pcm);
                    for (byte[] packet : packets) {
                        enqueuePacket(new OutgoingAudio(sessionId, sequence++,
                                System.currentTimeMillis(), audio.sampleRate, audio.channels,
                                CODEC_OPUS, packet));
                    }
                    return;
                } catch (IOException error) {
                    opusEncoder = null;
                }
            }
        }
        enqueuePacket(new OutgoingAudio(sessionId, sequence++, System.currentTimeMillis(),
                audio.sampleRate, audio.channels, CODEC_PCM16LE, audio.pcm));
    }

    private void enqueuePacket(OutgoingAudio packet) {
        if (packetQueue.offer(packet)) {
            return;
        }
        packetQueue.poll();
        packetQueue.offer(packet);
    }

    private void audioWriteLoop() {
        try {
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO);
        } catch (RuntimeException ignored) {
        }
        while (!Thread.currentThread().isInterrupted()) {
            try {
                OutgoingAudio packet = packetQueue.take();
                if (!isConnected()) {
                    continue;
                }
                writeFrame(PACKET_AUDIO, packet.sessionId, packet.sequence, packet.timestamp,
                        packet.sampleRate, packet.channels, packet.codec, packet.payload);
            } catch (InterruptedException error) {
                Thread.currentThread().interrupt();
                return;
            } catch (IOException error) {
                listener.onError("蓝牙音频发送失败：" + error.getMessage());
                packetQueue.clear();
            }
        }
    }

    private void connectInternal(BluetoothDevice device) {
        listener.onConnecting(device);
        try {
            BluetoothSocket newSocket = device.createRfcommSocketToServiceRecord(SERVICE_UUID);
            newSocket.connect();
            attachSocket(newSocket);
        } catch (IOException | SecurityException error) {
            listener.onError("蓝牙连接失败：" + error.getMessage());
            closeSocket();
        }
    }

    private void listenInternal() {
        try {
            BluetoothAdapter adapter = BluetoothAdapter.getDefaultAdapter();
            if (adapter == null) throw new IOException("当前设备不支持蓝牙");
            BluetoothServerSocket listenerSocket = adapter.listenUsingRfcommWithServiceRecord(
                    "AudioRouter", SERVICE_UUID);
            if (!listenRequested) {
                try { listenerSocket.close(); } catch (IOException ignored) { }
                return;
            }
            serverSocket = listenerSocket;
            while (serverSocket == listenerSocket) {
                BluetoothSocket accepted = listenerSocket.accept();
                if (serverSocket != listenerSocket) {
                    try { accepted.close(); } catch (IOException ignored) { }
                    return;
                }
                if (isConnected()) {
                    try { accepted.close(); } catch (IOException ignored) { }
                    continue;
                }
                BluetoothDevice device = accepted.getRemoteDevice();
                listener.onConnecting(device);
                attachSocket(accepted);
            }
        } catch (IOException | SecurityException error) {
            if (serverSocket != null) {
                serverSocket = null;
                listenRequested = false;
                listener.onError("蓝牙监听失败：" + error.getMessage());
            }
        }
    }

    private void attachSocket(BluetoothSocket newSocket) throws IOException {
        socket = newSocket;
        running = true;
        handshakeComplete = false;
        sendControl(buildHelloPayload());
        readLoop(newSocket);
    }

    private void readLoop(BluetoothSocket current) {
        try {
            InputStream input = current.getInputStream();
            byte[] header = new byte[HEADER_SIZE];
            while (running && current.isConnected()) {
                readFully(input, header, 0, HEADER_SIZE);
                validateHeader(header);
                int packetKind = header[5] & 0xff;
                int sampleRate = readInt(header, 28);
                int channels = header[32] & 0xff;
                int codec = header[33] & 0xff;
                int payloadLength = readInt(header, 36);
                if (payloadLength < 0 || payloadLength > MAX_PAYLOAD_SIZE) throw new IOException("负载长度非法");
                byte[] payload = new byte[payloadLength];
                readFully(input, payload, 0, payloadLength);
                if (packetKind == PACKET_AUDIO && receiveEnabled) {
                    if (codec == CODEC_PCM16LE) {
                        playPcm(payload, sampleRate, channels);
                    } else if (codec == CODEC_OPUS && opusNegotiated
                            && sampleRate == OpusAudioCodec.SAMPLE_RATE) {
                        long sourceSessionId = readLong(header, 8);
                        byte[] pcm = decodeOpus(sourceSessionId, payload, channels);
                        playPcm(pcm, sampleRate, channels);
                    }
                } else if (packetKind == PACKET_HELLO_ACK) {
                    int capabilities = validateControlPayload(payload, 2);
                    opusNegotiated = AudioQuality.supportsOpus(capabilities);
                    completeHandshake(current);
                } else if (packetKind == PACKET_CONTROL) {
                    if (isControlType(payload, 1)) {
                        int capabilities = validateControlPayload(payload, 1);
                        opusNegotiated = AudioQuality.supportsOpus(capabilities);
                        sendControl(buildHelloAckPayload());
                        completeHandshake(current);
                    } else {
                        applyIncomingControl(payload);
                    }
                }
            }
        } catch (IOException | RuntimeException error) {
            boolean notifyDisconnected = running;
            closeSocket();
            if (notifyDisconnected) {
                listener.onDisconnected("蓝牙连接已断开：" + error.getMessage());
            }
        }
    }

    private void completeHandshake(BluetoothSocket current) throws IOException {
        if (handshakeComplete) return;
        handshakeComplete = true;
        sendControl(new byte[]{1, 3, (byte) (sendEnabled ? 1 : 0)});
        sendControl(new byte[]{1, 4, (byte) (receiveEnabled ? 1 : 0)});
        sendControl(new byte[]{1, 5, (byte) sendVolumePercent});
        sendControl(new byte[]{1, 6, (byte) receiveVolumePercent});
        listener.onConnected(current.getRemoteDevice());
    }

    private boolean shouldSendOpus() {
        return opusNegotiated && !AudioQuality.BLUETOOTH_PCM.equals(codecMode);
    }

    private void applyIncomingControl(byte[] payload) {
        if (payload.length != 3 || payload[0] != 1) return;
        int value = payload[2] & 0xff;
        switch (payload[1] & 0xff) {
            case 3:
                remoteSendEnabled = value != 0;
                break;
            case 4:
                receiveEnabled = value != 0;
                break;
            case 5:
                sendVolumePercent = clamp(value);
                break;
            case 6:
                receiveVolumePercent = clamp(value);
                receiveVolume = PcmAudio.volumeToGain(receiveVolumePercent);
                break;
            default:
                break;
        }
    }

    private byte[] decodeOpus(long sourceSessionId, byte[] packet, int channels) throws IOException {
        synchronized (codecLock) {
            DecoderState state = opusDecoders.get(sourceSessionId);
            if (state == null || state.channels != channels) {
                state = new DecoderState(channels);
                opusDecoders.put(sourceSessionId, state);
            }
            return state.decoder.decode(packet);
        }
    }

    private void playPcm(byte[] pcm, int sampleRate, int channels) {
        if (sampleRate <= 0 || (channels != 1 && channels != 2) || pcm.length == 0) return;
        byte[] output = PcmAudio.applyPlaybackGain(pcm, receiveVolume);
        int frameMs = Math.max(1, output.length * 1000
                / (sampleRate * channels * 2));
        synchronized (playbackQueue) {
            playbackQueue.addLast(output);
            playbackQueuedMs += frameMs;
            while (playbackQueuedMs > audioBufferMs + frameMs && playbackQueue.size() > 1) {
                byte[] dropped = playbackQueue.removeFirst();
                playbackQueuedMs -= Math.max(1, dropped.length * 1000
                        / (sampleRate * channels * 2));
            }
            if (playbackQueuedMs < audioBufferMs) return;
            while (!playbackQueue.isEmpty()) {
                byte[] queued = playbackQueue.removeFirst();
                playbackQueuedMs -= Math.max(1, queued.length * 1000
                        / (sampleRate * channels * 2));
                writePcm(queued, sampleRate, channels);
            }
            return;
        }
    }

    private void writePcm(byte[] output, int sampleRate, int channels) {
        int channelMask = channels == 1 ? AudioFormat.CHANNEL_OUT_MONO : AudioFormat.CHANNEL_OUT_STEREO;
        if (audioTrack == null || audioTrackSampleRate != sampleRate || audioTrackChannels != channels) {
            closeAudioTrack();
            int minBuffer = AudioTrack.getMinBufferSize(sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT);
            if (minBuffer <= 0) return;
            audioTrack = new AudioTrack.Builder()
                    .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                    .setAudioFormat(new AudioFormat.Builder().setSampleRate(sampleRate).setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(channelMask).build())
                    .setBufferSizeInBytes(Math.max(minBuffer, output.length))
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .build();
            audioTrackSampleRate = sampleRate;
            audioTrackChannels = channels;
            audioTrack.setVolume(1.0f);
            audioTrack.play();
        }
        audioTrack.write(output, 0, output.length, AudioTrack.WRITE_BLOCKING);
    }

    private void sendControl(byte[] payload) throws IOException {
        writeFrame(PACKET_CONTROL, 0, 0, 0, 0, 0, 0, payload);
    }

    private void sendControlIfReady(byte[] payload) throws IOException {
        if (handshakeComplete) sendControl(payload);
    }

    private byte[] buildHelloPayload() {
        return buildHelloPayloadWithType(1);
    }

    private byte[] buildHelloAckPayload() {
        return buildHelloPayloadWithType(2);
    }

    private byte[] buildHelloPayloadWithType(int type) {
        byte[] name = "AudioRouter Android 16".getBytes(StandardCharsets.UTF_8);
        ByteBuffer payload = ByteBuffer.allocate(2 + 8 + 2 + name.length + 4)
                .order(ByteOrder.LITTLE_ENDIAN);
        payload.put((byte) 1).put((byte) type).putLong(sessionId)
                .putShort((short) name.length).put(name)
                .putInt(AudioQuality.buildBluetoothCapabilities(codecMode));
        return payload.array();
    }

    private static boolean isControlType(byte[] payload, int type) {
        return payload.length >= 2 && payload[0] == 1 && payload[1] == (byte) type;
    }

    private static int validateControlPayload(byte[] payload, int expectedType) throws IOException {
        if (payload.length < 17 || !isControlType(payload, expectedType)) {
            throw new IOException("蓝牙握手消息无效");
        }
        int nameLength = ByteBuffer.wrap(payload, 10, 2).order(ByteOrder.LITTLE_ENDIAN).getShort() & 0xffff;
        if (nameLength == 0 || nameLength > 128 || payload.length != 16 + nameLength) {
            throw new IOException("蓝牙握手设备名称无效");
        }
        return readInt(payload, 12 + nameLength);
    }

    private void writeFrame(int kind, long frameSessionId, int frameSequence, long timestamp,
                            int sampleRate, int channels, int codec, byte[] payload) throws IOException {
        if (payload.length > MAX_PAYLOAD_SIZE) throw new IOException("负载过大");
        BluetoothSocket current = socket;
        if (current == null || !current.isConnected()) throw new IOException("蓝牙连接未建立");
        ByteBuffer frame = ByteBuffer.allocate(HEADER_SIZE + payload.length).order(ByteOrder.LITTLE_ENDIAN);
        frame.put(MAGIC).put(PROTOCOL_VERSION).put((byte) kind).put((byte) 0).put((byte) 0)
                .putLong(frameSessionId).putInt(frameSequence).putLong(timestamp)
                .putInt(sampleRate).put((byte) channels).put((byte) codec).putShort((short) 0)
                .putInt(payload.length).put(payload);
        synchronized (writeLock) {
            OutputStream output = current.getOutputStream();
            output.write(frame.array());
        }
    }

    private static void validateHeader(byte[] header) throws IOException {
        for (int i = 0; i < MAGIC.length; i++) if (header[i] != MAGIC[i]) throw new IOException("无效协议帧");
        if (header[4] != PROTOCOL_VERSION) throw new IOException("协议版本不兼容");
    }

    private static int readInt(byte[] bytes, int offset) {
        return ByteBuffer.wrap(bytes, offset, 4).order(ByteOrder.LITTLE_ENDIAN).getInt();
    }

    private static long readLong(byte[] bytes, int offset) {
        return ByteBuffer.wrap(bytes, offset, 8).order(ByteOrder.LITTLE_ENDIAN).getLong();
    }

    private static void readFully(InputStream input, byte[] buffer, int offset, int length) throws IOException {
        int read = 0;
        while (read < length) {
            int count = input.read(buffer, offset + read, length - read);
            if (count < 0) throw new IOException("连接提前关闭");
            read += count;
        }
    }

    private void closeSocket() {
        running = false;
        handshakeComplete = false;
        closeAudioTrack();
        BluetoothSocket current = socket;
        socket = null;
        if (current != null) {
            try { current.close(); } catch (IOException ignored) { }
        }
    }

    private void closeAudioTrack() {
        if (audioTrack != null) {
            try { audioTrack.stop(); } catch (IllegalStateException ignored) { }
            audioTrack.release();
            audioTrack = null;
            audioTrackSampleRate = 0;
            audioTrackChannels = 0;
        }
    }

    private static int clamp(int value) { return Math.max(0, Math.min(100, value)); }

    private static final class DecoderState {
        final int channels;
        final OpusAudioCodec.Decoder decoder;

        DecoderState(int channels) throws IOException {
            this.channels = channels;
            decoder = new OpusAudioCodec.Decoder(channels);
        }
    }

    private static final class CapturedAudio {
        final int channels;
        final int sampleRate;
        final byte[] pcm;

        CapturedAudio(int sampleRate, int channels, byte[] pcm) {
            this.sampleRate = sampleRate;
            this.channels = channels;
            this.pcm = pcm;
        }
    }

    private static final class OutgoingAudio {
        final long sessionId;
        final int sequence;
        final long timestamp;
        final int sampleRate;
        final int channels;
        final int codec;
        final byte[] payload;

        OutgoingAudio(long sessionId, int sequence, long timestamp, int sampleRate,
                      int channels, int codec, byte[] payload) {
            this.sessionId = sessionId;
            this.sequence = sequence;
            this.timestamp = timestamp;
            this.sampleRate = sampleRate;
            this.channels = channels;
            this.codec = codec;
            this.payload = payload;
        }
    }
}
