package com.audiorouter.app;

import android.Manifest;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.AudioManager;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;
import android.content.pm.PackageManager;
import android.util.Base64;
import android.net.wifi.WifiManager;

import java.util.Arrays;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class AudioRouterService extends Service {
    public static final String ACTION_CONNECT = "com.audiorouter.app.CONNECT";
    public static final String ACTION_BLUETOOTH_CONNECT = "com.audiorouter.app.BLUETOOTH_CONNECT";
    public static final String ACTION_DISCONNECT = "com.audiorouter.app.DISCONNECT";
    public static final String ACTION_STATUS = "com.audiorouter.app.STATUS";
    public static final String ACTION_CAPTURE = "com.audiorouter.app.CAPTURE";
    public static final String ACTION_STOP = "com.audiorouter.app.STOP";
    public static final String ACTION_STOP_CAPTURE = "com.audiorouter.app.STOP_CAPTURE";
    public static final String ACTION_LISTEN = "com.audiorouter.app.LISTEN";
    public static final String ACTION_SET_LAN_LISTEN = "com.audiorouter.app.SET_LAN_LISTEN";
    public static final String ACTION_SET_BLUETOOTH_LISTEN = "com.audiorouter.app.SET_BLUETOOTH_LISTEN";
    public static final String ACTION_SEND_ENABLED = "com.audiorouter.app.SEND_ENABLED";
    public static final String ACTION_RECEIVE_ENABLED = "com.audiorouter.app.RECEIVE_ENABLED";
    public static final String ACTION_SEND_VOLUME = "com.audiorouter.app.SEND_VOLUME";
    public static final String ACTION_RECEIVE_VOLUME = "com.audiorouter.app.RECEIVE_VOLUME";
    public static final String ACTION_SYSTEM_VOLUME_ENABLED = "com.audiorouter.app.SYSTEM_VOLUME_ENABLED";
    public static final String ACTION_SET_LAN_AUDIO_QUALITY =
            "com.audiorouter.app.SET_LAN_AUDIO_QUALITY";
    public static final String ACTION_SET_BLUETOOTH_AUDIO_QUALITY =
            "com.audiorouter.app.SET_BLUETOOTH_AUDIO_QUALITY";
    public static final String ACTION_SET_AUDIO_BUFFER =
            "com.audiorouter.app.SET_AUDIO_BUFFER";
    public static final String EXTRA_HOST = "host";
    public static final String EXTRA_DEVICE = "device";
    public static final String EXTRA_STATUS = "status";
    public static final String EXTRA_PORT = "port";
    public static final String EXTRA_RESULT_CODE = "result_code";
    public static final String EXTRA_RESULT_DATA = "result_data";
    public static final String EXTRA_ENABLED = "enabled";
    public static final String EXTRA_VOLUME = "volume";
    public static final String EXTRA_TRANSPORT = "transport";
    public static final String EXTRA_DEVICE_NAME = "device_name";
    public static final String EXTRA_DEVICE_ID = "device_id";
    public static final String EXTRA_CAPTURING = "capturing";
    public static final String EXTRA_CODEC_MODE = "codec_mode";
    public static final String EXTRA_OPUS_BITRATE_KBPS = "opus_bitrate_kbps";
    public static final String EXTRA_PCM_CHANNELS = "pcm_channels";
    public static final String EXTRA_BUFFER_MS = "buffer_ms";
    public static final String EXTRA_CONNECTED = "connected";
    public static final String EXTRA_SEND_ENABLED = "send_enabled";
    public static final String EXTRA_REMOTE_SEND_ENABLED = "remote_send_enabled";
    public static final String EXTRA_LAN_CONNECTED = "lan_connected";
    public static final String EXTRA_LAN_LISTEN_ENABLED = "lan_listen_enabled_state";
    public static final String EXTRA_LAN_NAME = "lan_name";
    public static final String EXTRA_LAN_CONNECTED_AT = "lan_connected_at";
    public static final String EXTRA_LAN_DEVICE_ID = "lan_device_id";
    public static final String EXTRA_BLUETOOTH_CONNECTED = "bluetooth_connected";
    public static final String EXTRA_BLUETOOTH_LISTEN_ENABLED = "bluetooth_listen_enabled_state";
    public static final String EXTRA_BLUETOOTH_NAME = "bluetooth_name";
    public static final String EXTRA_BLUETOOTH_ADDRESS = "bluetooth_address";
    public static final String EXTRA_BLUETOOTH_CONNECTED_AT = "bluetooth_connected_at";
    public static final String EXTRA_BLUETOOTH_DEVICE_ID = "bluetooth_device_id";

    static final String TRANSPORT_LAN = "lan";
    static final String TRANSPORT_BLUETOOTH = "bluetooth";
    static final String SETTING_SEND_ENABLED = "send_enabled";
    static final String SETTING_SEND_VOLUME = "send_volume";
    static final String SETTING_FOLLOW_SYSTEM_VOLUME = "follow_system_volume";
    static final String SETTING_RECEIVE_ENABLED = "receive_enabled";
    static final String SETTING_RECEIVE_VOLUME = "receive_volume";

    private static final String CHANNEL_ID = "audiorouter-media";
    private static final String PREFS_NAME = "audiorouter-settings";
    private static final String PREF_SYSTEM_VOLUME_ENABLED = "system_volume_enabled";
    private static final String PREF_LAN_LISTEN_ENABLED = "lan_listen_enabled";
    private static final String PREF_BLUETOOTH_LISTEN_ENABLED = "bluetooth_listen_enabled";
    private static final String PREF_RESTORE_CONNECTION_STATE = "restore_connection_state";
    private static final String PREF_AUTO_CONNECT = "auto_connect";
    private static final String PREF_AUTO_CONNECT_ATTEMPTS = "auto_connect_attempts";
    private static final String PREF_AUTO_CONNECT_INTERVAL_SECONDS = "auto_connect_interval_seconds";
    private static final String PREF_BLUETOOTH_DEVICE_ADDRESS = "bluetooth_device_address";
    private static final String PREF_LAN_HOST = "lan_host";
    private static final String PREF_LAN_PORT = "lan_port";
    static final String PREF_LAN_CODEC_MODE = "lan_codec_mode";
    static final String PREF_LAN_OPUS_BITRATE_KBPS = "lan_opus_bitrate_kbps";
    static final String PREF_LAN_PCM_CHANNELS = "lan_pcm_channels";
    static final String PREF_BLUETOOTH_CODEC_MODE = "bluetooth_codec_mode";
    static final String PREF_AUDIO_BUFFER_MS = "audio_buffer_ms";
    static final int DEFAULT_AUDIO_BUFFER_MS = 20;
    private static final int CAPTURE_SAMPLE_RATE = 48_000;
    private static final int CAPTURE_CHANNELS = 2;
    private static final int CAPTURE_FRAME_SAMPLES = 960;
    private static final int CAPTURE_BLOCK_BYTES = CAPTURE_CHANNELS * Short.BYTES;
    private static final int CAPTURE_FRAME_BYTES = CAPTURE_FRAME_SAMPLES * CAPTURE_BLOCK_BYTES;
    private static final int NOTIFICATION_ID = 1001;
    private LanSession lanSession;
    private BluetoothSession bluetoothSession;
    private volatile DeviceAudioSettings lanAudioSettings;
    private volatile DeviceAudioSettings bluetoothAudioSettings;
    private volatile boolean bluetoothMode;
    private Thread captureThread;
    private volatile boolean capturing;
    private volatile boolean lanConnecting;
    private volatile int lanConnectionGeneration;
    private volatile int bluetoothConnectionGeneration;
    private volatile String lanPeerName = "局域网设备";
    private volatile String lanDeviceId = "";
    private volatile String bluetoothPeerName = "蓝牙设备";
    private volatile String bluetoothDeviceId = "";
    private volatile String bluetoothPeerAddress = "";
    private volatile String lanCodecMode = AudioQuality.LAN_COMPRESSED;
    private volatile int lanOpusBitrateKbps = AudioQuality.DEFAULT_LAN_OPUS_BITRATE_KBPS;
    private volatile int lanPcmChannels = AudioQuality.DEFAULT_PCM_CHANNELS;
    private volatile String bluetoothCodecMode = AudioQuality.BLUETOOTH_AUTO;
    private volatile int audioBufferMs = DEFAULT_AUDIO_BUFFER_MS;
    private volatile long lanConnectedAtElapsedRealtime;
    private volatile long bluetoothConnectedAtElapsedRealtime;
    private volatile long capturedFrames;
    private volatile long sentFrames;
    private volatile long silentFrames;
    private MediaProjection mediaProjection;
    private AudioRecord audioRecord;
    private AudioManager audioManager;
    private PowerManager.WakeLock transferWakeLock;
    private WifiManager.WifiLock lanWifiLock;
    private volatile boolean bluetoothConnectionPending;
    private boolean configuredStateRestored;
    private final ExecutorService controlExecutor = Executors.newSingleThreadExecutor();

    @Override
    public void onCreate() {
        super.onCreate();
        audioManager = getSystemService(AudioManager.class);
        PowerManager powerManager = getSystemService(PowerManager.class);
        transferWakeLock = powerManager.newWakeLock(
                PowerManager.PARTIAL_WAKE_LOCK, getPackageName() + ":audio-transfer");
        transferWakeLock.setReferenceCounted(false);
        WifiManager wifiManager = getApplicationContext().getSystemService(WifiManager.class);
        if (wifiManager != null) {
            lanWifiLock = wifiManager.createWifiLock(
                    WifiManager.WIFI_MODE_FULL, getPackageName() + ":lan-audio-transfer");
            lanWifiLock.setReferenceCounted(false);
        }
        loadAudioQualitySettings();
        lanAudioSettings = loadAudioSettings(TRANSPORT_LAN, "");
        bluetoothAudioSettings = loadAudioSettings(TRANSPORT_BLUETOOTH, "");
        lanSession = new LanSession(new LanSession.Listener() {
            @Override public void onConnected(String deviceName) {
                String identity = lanSession.peerIdentity();
                lanPeerName = deviceDisplayName(TRANSPORT_LAN, identity,
                        friendlyPeerName(deviceName, "局域网设备"));
                if (!identity.isEmpty()) {
                    lanDeviceId = identity;
                    lanAudioSettings = loadAudioSettings(TRANSPORT_LAN, identity);
                }
                lanConnectedAtElapsedRealtime = android.os.SystemClock.elapsedRealtime();
                updateTransferWakeLocks();
                sendControl(() -> applyDeviceSettings(TRANSPORT_LAN));
                publishStatus("局域网已连接：" + lanPeerName);
            }
            @Override public void onDisconnected() {
                lanConnectedAtElapsedRealtime = 0;
                updateTransferWakeLocks();
                lanPeerName = "局域网设备";
                lanDeviceId = "";
                publishStatus("局域网连接已断开");
            }
        });
        bluetoothSession = new BluetoothSession(new BluetoothSession.Listener() {
            @Override public void onConnecting(BluetoothDevice device) {
                bluetoothMode = true;
                bluetoothConnectionPending = true;
                updateTransferWakeLocks();
                bluetoothDeviceId = bluetoothDeviceAddress(device);
                bluetoothPeerName = deviceDisplayName(TRANSPORT_BLUETOOTH,
                        bluetoothDeviceId, bluetoothDeviceName(device));
                if (!bluetoothDeviceId.isEmpty()) {
                    bluetoothAudioSettings = loadAudioSettings(
                            TRANSPORT_BLUETOOTH, bluetoothDeviceId);
                    bluetoothSession.setInitialSendEnabled(bluetoothAudioSettings.sendEnabled);
                }
                publishStatus("正在连接蓝牙设备：" + bluetoothPeerName);
            }
            @Override public void onConnected(BluetoothDevice device) {
                bluetoothMode = true;
                bluetoothConnectionPending = false;
                bluetoothPeerAddress = bluetoothDeviceAddress(device);
                bluetoothPeerName = deviceDisplayName(TRANSPORT_BLUETOOTH,
                        bluetoothPeerAddress, bluetoothDeviceName(device));
                bluetoothDeviceId = bluetoothPeerAddress;
                if (!bluetoothDeviceId.isEmpty()) {
                    bluetoothAudioSettings = loadAudioSettings(
                            TRANSPORT_BLUETOOTH, bluetoothDeviceId);
                }
                bluetoothConnectedAtElapsedRealtime = android.os.SystemClock.elapsedRealtime();
                updateTransferWakeLocks();
                sendControl(() -> applyDeviceSettings(TRANSPORT_BLUETOOTH));
                publishStatus("蓝牙已连接：" + bluetoothPeerName);
            }
            @Override public void onDisconnected(String message) {
                bluetoothMode = false;
                bluetoothConnectionPending = false;
                bluetoothConnectedAtElapsedRealtime = 0;
                bluetoothPeerAddress = "";
                bluetoothDeviceId = "";
                bluetoothPeerName = "蓝牙设备";
                updateTransferWakeLocks();
                publishStatus(message);
            }
            @Override public void onError(String message) {
                bluetoothConnectionPending = false;
                updateTransferWakeLocks();
                publishStatus(message);
            }
        });
        try {
            lanSession.setAudioQuality(lanCodecMode, lanOpusBitrateKbps);
            bluetoothSession.setAudioQuality(bluetoothCodecMode);
            lanSession.setAudioBufferMs(audioBufferMs);
            bluetoothSession.setAudioBufferMs(audioBufferMs);
        } catch (Exception error) {
            publishStatus("初始化音频质量设置失败：" + describe(error));
        }
        createNotificationChannel();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, createNotification(),
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(NOTIFICATION_ID, createNotification());
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || intent.getAction() == null) {
            restoreConfiguredState(true);
            return START_STICKY;
        }
        String action = intent.getAction();
        try {
            switch (action) {
            case ACTION_LISTEN:
                if (!configuredStateRestored) restoreConfiguredState(true);
                publishStatus("音频路由服务已就绪");
                break;
            case ACTION_SET_LAN_LISTEN:
                setLanListening(intent.getBooleanExtra(EXTRA_ENABLED, false));
                break;
            case ACTION_SET_BLUETOOTH_LISTEN:
                setBluetoothListening(intent.getBooleanExtra(EXTRA_ENABLED, false));
                break;
            case ACTION_SET_LAN_AUDIO_QUALITY:
                setLanAudioQuality(intent);
                break;
            case ACTION_SET_BLUETOOTH_AUDIO_QUALITY:
                setBluetoothAudioQuality(intent);
                break;
            case ACTION_SET_AUDIO_BUFFER:
                setAudioBuffer(intent.getIntExtra(EXTRA_BUFFER_MS, DEFAULT_AUDIO_BUFFER_MS));
                break;
            case ACTION_CONNECT:
                if (!isLanListeningEnabled()) {
                    publishStatus("请先启用局域网监听");
                    break;
                }
                bluetoothMode = false;
                String host = intent.getStringExtra(EXTRA_HOST);
                int port = intent.getIntExtra(EXTRA_PORT, LanSession.DEFAULT_PORT);
                String name = intent.getStringExtra(EXTRA_DEVICE_NAME);
                if (isAutoConnectEnabled()) connectWithRetry(host, port, name);
                else connect(host, port, name);
                break;
            case ACTION_BLUETOOTH_CONNECT:
                if (!isBluetoothListeningEnabled()) {
                    publishStatus("请先启用蓝牙监听");
                    break;
                }
                BluetoothDevice device = getBluetoothDevice(intent);
                if (device != null) {
                    bluetoothMode = true;
                    bluetoothConnectionGeneration++;
                    bluetoothDeviceId = bluetoothDeviceAddress(device);
                    settings().edit().putString(PREF_BLUETOOTH_DEVICE_ADDRESS,
                            bluetoothDeviceId).apply();
                    String selectedName = friendlyPeerName(intent.getStringExtra(EXTRA_DEVICE_NAME),
                            bluetoothDeviceName(device));
                    rememberDeviceName(TRANSPORT_BLUETOOTH, bluetoothDeviceId, selectedName);
                    bluetoothPeerName = selectedName;
                    bluetoothAudioSettings = loadAudioSettings(
                            TRANSPORT_BLUETOOTH, bluetoothDeviceId);
                    bluetoothConnectedAtElapsedRealtime = 0;
                    updateTransferWakeLocks();
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        startForeground(NOTIFICATION_ID, createNotification(),
                                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                                        | ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE);
                    }
                    if (isAutoConnectEnabled()) {
                        connectBluetoothWithRetry(device);
                    } else {
                        bluetoothSession.connect(device, bluetoothAudioSettings.sendEnabled);
                    }
                }
                break;
            case ACTION_DISCONNECT:
                disconnectTransport(intent.getStringExtra(EXTRA_TRANSPORT));
                break;
            case ACTION_CAPTURE:
                startCapture(intent);
                break;
            case ACTION_SEND_ENABLED:
                updateDeviceSetting(intent, SETTING_SEND_ENABLED,
                        intent.getBooleanExtra(EXTRA_ENABLED, false));
                break;
            case ACTION_RECEIVE_ENABLED:
                updateDeviceSetting(intent, SETTING_RECEIVE_ENABLED,
                        intent.getBooleanExtra(EXTRA_ENABLED, false));
                break;
            case ACTION_SEND_VOLUME:
                updateDeviceSetting(intent, SETTING_SEND_VOLUME,
                        clamp(intent.getIntExtra(EXTRA_VOLUME, 100)));
                break;
            case ACTION_SYSTEM_VOLUME_ENABLED:
                updateDeviceSetting(intent, SETTING_FOLLOW_SYSTEM_VOLUME,
                        intent.getBooleanExtra(EXTRA_ENABLED, false));
                break;
            case ACTION_RECEIVE_VOLUME:
                updateDeviceSetting(intent, SETTING_RECEIVE_VOLUME,
                        clamp(intent.getIntExtra(EXTRA_VOLUME, 100)));
                break;
            case ACTION_STOP:
            case ACTION_STOP_CAPTURE:
                stopCapture();
                publishStatus("手机音频录制已停止");
                break;
                default:
                    break;
            }
        } catch (RuntimeException error) {
            publishStatus("服务操作失败：" + describe(error));
            stopCapture();
        }
        return START_STICKY;
    }

    @Override
    public void onTaskRemoved(Intent rootIntent) {
        // Keep the foreground audio sessions alive when the activity is swiped away.
        super.onTaskRemoved(rootIntent);
    }

    private android.content.SharedPreferences settings() {
        return getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
    }

    private void loadAudioQualitySettings() {
        android.content.SharedPreferences preferences = settings();
        audioBufferMs = clampAudioBuffer(preferences.getInt(
                PREF_AUDIO_BUFFER_MS, DEFAULT_AUDIO_BUFFER_MS));
        String storedLanMode = preferences.getString(PREF_LAN_CODEC_MODE,
                AudioQuality.LAN_UNCOMPRESSED);
        lanCodecMode = AudioQuality.LAN_UNCOMPRESSED.equals(storedLanMode)
                ? AudioQuality.LAN_UNCOMPRESSED : AudioQuality.LAN_COMPRESSED;
        lanOpusBitrateKbps = clampOpusBitrate(preferences.getInt(
                PREF_LAN_OPUS_BITRATE_KBPS, AudioQuality.DEFAULT_LAN_OPUS_BITRATE_KBPS));
        int storedPcmChannels = preferences.getInt(
                PREF_LAN_PCM_CHANNELS, AudioQuality.DEFAULT_PCM_CHANNELS);
        lanPcmChannels = storedPcmChannels == 1 || storedPcmChannels == 2
                ? storedPcmChannels : AudioQuality.DEFAULT_PCM_CHANNELS;
        String storedBluetoothMode = preferences.getString(PREF_BLUETOOTH_CODEC_MODE,
                AudioQuality.BLUETOOTH_AUTO);
        if (AudioQuality.BLUETOOTH_OPUS.equals(storedBluetoothMode)) {
            bluetoothCodecMode = AudioQuality.BLUETOOTH_OPUS;
        } else if (AudioQuality.BLUETOOTH_PCM.equals(storedBluetoothMode)) {
            bluetoothCodecMode = AudioQuality.BLUETOOTH_PCM;
        } else {
            bluetoothCodecMode = AudioQuality.BLUETOOTH_AUTO;
        }
    }

    private void setAudioBuffer(int bufferMs) {
        if (blockActiveAudioSettingsChange()) return;
        audioBufferMs = clampAudioBuffer(bufferMs);
        settings().edit().putInt(PREF_AUDIO_BUFFER_MS, audioBufferMs).apply();
        lanSession.setAudioBufferMs(audioBufferMs);
        bluetoothSession.setAudioBufferMs(audioBufferMs);
        publishStatus("音频缓冲已更新为 " + audioBufferMs + " 毫秒");
    }

    static int savedAudioBufferMs(android.content.SharedPreferences preferences) {
        return clampAudioBuffer(preferences.getInt(PREF_AUDIO_BUFFER_MS, DEFAULT_AUDIO_BUFFER_MS));
    }

    private static int clampAudioBuffer(int bufferMs) {
        return Math.max(20, Math.min(240, bufferMs));
    }

    private void setLanAudioQuality(Intent intent) {
        if (blockActiveAudioSettingsChange()) return;
        String mode = intent.getStringExtra(EXTRA_CODEC_MODE);
        lanCodecMode = AudioQuality.LAN_UNCOMPRESSED.equals(mode)
                ? AudioQuality.LAN_UNCOMPRESSED : AudioQuality.LAN_COMPRESSED;
        lanOpusBitrateKbps = clampOpusBitrate(intent.getIntExtra(
                EXTRA_OPUS_BITRATE_KBPS, lanOpusBitrateKbps));
        int channels = intent.getIntExtra(EXTRA_PCM_CHANNELS, lanPcmChannels);
        lanPcmChannels = channels == 1 || channels == 2 ? channels : AudioQuality.DEFAULT_PCM_CHANNELS;
        settings().edit()
                .putString(PREF_LAN_CODEC_MODE, lanCodecMode)
                .putInt(PREF_LAN_OPUS_BITRATE_KBPS, lanOpusBitrateKbps)
                .putInt(PREF_LAN_PCM_CHANNELS, lanPcmChannels)
                .apply();
        try {
            lanSession.setAudioQuality(lanCodecMode, lanOpusBitrateKbps);
            publishStatus("局域网音频质量已更新");
        } catch (Exception error) {
            publishStatus("局域网音频质量更新失败：" + describe(error));
        }
    }

    private void setBluetoothAudioQuality(Intent intent) {
        if (blockActiveAudioSettingsChange()) return;
        String mode = intent.getStringExtra(EXTRA_CODEC_MODE);
        if (AudioQuality.BLUETOOTH_OPUS.equals(mode)) {
            bluetoothCodecMode = AudioQuality.BLUETOOTH_OPUS;
        } else if (AudioQuality.BLUETOOTH_PCM.equals(mode)) {
            bluetoothCodecMode = AudioQuality.BLUETOOTH_PCM;
        } else {
            bluetoothCodecMode = AudioQuality.BLUETOOTH_AUTO;
        }
        settings().edit().putString(PREF_BLUETOOTH_CODEC_MODE, bluetoothCodecMode).apply();
        try {
            bluetoothSession.setAudioQuality(bluetoothCodecMode);
            publishStatus("蓝牙音频协议已更新");
        } catch (Exception error) {
            publishStatus("蓝牙音频协议更新失败：" + describe(error));
        }
    }

    private boolean blockActiveAudioSettingsChange() {
        if (!capturing && !lanSession.isConnected() && !bluetoothSession.isConnected()) return false;
        settings().edit()
                .putInt(PREF_AUDIO_BUFFER_MS, audioBufferMs)
                .putString(PREF_LAN_CODEC_MODE, lanCodecMode)
                .putInt(PREF_LAN_OPUS_BITRATE_KBPS, lanOpusBitrateKbps)
                .putInt(PREF_LAN_PCM_CHANNELS, lanPcmChannels)
                .putString(PREF_BLUETOOTH_CODEC_MODE, bluetoothCodecMode)
                .apply();
        publishStatus("请关闭传输后再调整设置");
        return true;
    }

    private static int clampOpusBitrate(int bitrateKbps) {
        return Math.max(6, Math.min(450, bitrateKbps));
    }

    static String devicePreferenceKey(String transport, String identity, String setting) {
        String normalized = identity == null ? "" : identity.trim().toLowerCase(java.util.Locale.ROOT);
        String encodedIdentity = normalized.isEmpty()
                ? "default"
                : Base64.encodeToString(normalized.getBytes(StandardCharsets.UTF_8),
                        Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
        return "device." + transport + "." + encodedIdentity + "." + setting;
    }

    private DeviceAudioSettings loadAudioSettings(String transport, String identity) {
        android.content.SharedPreferences preferences = settings();
        String keyIdentity = devicePreferenceKey(transport, identity, SETTING_SEND_ENABLED);
        String defaultIdentity = devicePreferenceKey(transport, "", SETTING_SEND_ENABLED);
        DeviceAudioSettings value = new DeviceAudioSettings();
        value.sendEnabled = preferences.getBoolean(
                keyIdentity, preferences.getBoolean(defaultIdentity, true));
        value.sendVolume = preferences.getInt(
                devicePreferenceKey(transport, identity, SETTING_SEND_VOLUME),
                preferences.getInt(devicePreferenceKey(transport, "", SETTING_SEND_VOLUME), 100));
        value.followSystemVolume = preferences.getBoolean(
                devicePreferenceKey(transport, identity, SETTING_FOLLOW_SYSTEM_VOLUME),
                preferences.getBoolean(
                        devicePreferenceKey(transport, "", SETTING_FOLLOW_SYSTEM_VOLUME),
                        preferences.getBoolean(PREF_SYSTEM_VOLUME_ENABLED, false)));
        value.receiveEnabled = preferences.getBoolean(
                devicePreferenceKey(transport, identity, SETTING_RECEIVE_ENABLED),
                preferences.getBoolean(devicePreferenceKey(
                        transport, "", SETTING_RECEIVE_ENABLED), true));
        value.receiveVolume = preferences.getInt(
                devicePreferenceKey(transport, identity, SETTING_RECEIVE_VOLUME),
                preferences.getInt(devicePreferenceKey(
                        transport, "", SETTING_RECEIVE_VOLUME), 100));
        preferences.edit()
                .putBoolean(devicePreferenceKey(transport, identity, SETTING_SEND_ENABLED), value.sendEnabled)
                .putInt(devicePreferenceKey(transport, identity, SETTING_SEND_VOLUME), value.sendVolume)
                .putBoolean(devicePreferenceKey(transport, identity, SETTING_FOLLOW_SYSTEM_VOLUME), value.followSystemVolume)
                .putBoolean(devicePreferenceKey(transport, identity, SETTING_RECEIVE_ENABLED), value.receiveEnabled)
                .putInt(devicePreferenceKey(transport, identity, SETTING_RECEIVE_VOLUME), value.receiveVolume)
                .apply();
        return value;
    }

    private String targetTransport(Intent intent) {
        String transport = intent.getStringExtra(EXTRA_TRANSPORT);
        if (TRANSPORT_LAN.equals(transport) || TRANSPORT_BLUETOOTH.equals(transport)) {
            return transport;
        }
        return bluetoothMode ? TRANSPORT_BLUETOOTH : TRANSPORT_LAN;
    }

    private DeviceAudioSettings audioSettings(String transport) {
        return TRANSPORT_BLUETOOTH.equals(transport)
                ? bluetoothAudioSettings : lanAudioSettings;
    }

    private void updateDeviceSetting(Intent intent, String setting, Object value) {
        String transport = targetTransport(intent);
        String activeIdentity = TRANSPORT_BLUETOOTH.equals(transport)
                ? bluetoothDeviceId : lanDeviceId;
        String requestedIdentity = intent.getStringExtra(EXTRA_DEVICE_ID);
        String identity = requestedIdentity == null ? activeIdentity : requestedIdentity;
        boolean activeDevice = identity.equals(activeIdentity);
        DeviceAudioSettings deviceSettings = activeDevice
                ? audioSettings(transport) : loadAudioSettings(transport, identity);
        android.content.SharedPreferences.Editor editor = settings().edit();
        switch (setting) {
            case SETTING_SEND_ENABLED:
                deviceSettings.sendEnabled = (Boolean) value;
                editor.putBoolean(devicePreferenceKey(transport, identity, setting), deviceSettings.sendEnabled);
                break;
            case SETTING_SEND_VOLUME:
                deviceSettings.sendVolume = (Integer) value;
                editor.putInt(devicePreferenceKey(transport, identity, setting), deviceSettings.sendVolume);
                break;
            case SETTING_FOLLOW_SYSTEM_VOLUME:
                deviceSettings.followSystemVolume = (Boolean) value;
                editor.putBoolean(devicePreferenceKey(transport, identity, setting), deviceSettings.followSystemVolume);
                break;
            case SETTING_RECEIVE_ENABLED:
                deviceSettings.receiveEnabled = (Boolean) value;
                editor.putBoolean(devicePreferenceKey(transport, identity, setting), deviceSettings.receiveEnabled);
                break;
            case SETTING_RECEIVE_VOLUME:
                deviceSettings.receiveVolume = (Integer) value;
                editor.putInt(devicePreferenceKey(transport, identity, setting), deviceSettings.receiveVolume);
                break;
            default:
                return;
        }
        editor.apply();
        if (activeDevice) {
            sendControl(() -> applyDeviceSetting(transport, setting));
        }
        publishStatus("已更新" + (TRANSPORT_BLUETOOTH.equals(transport) ? "蓝牙" : "局域网")
                + "设备设置");
    }

    private void applyDeviceSettings(String transport) throws Exception {
        applyDeviceSetting(transport, SETTING_SEND_ENABLED);
        applyDeviceSetting(transport, SETTING_RECEIVE_ENABLED);
        applyDeviceSetting(transport, SETTING_RECEIVE_VOLUME);
    }

    private void applyDeviceSetting(String transport, String setting) throws Exception {
        DeviceAudioSettings deviceSettings = audioSettings(transport);
        if (TRANSPORT_BLUETOOTH.equals(transport)) {
            if (!bluetoothSession.isConnected()) return;
            if (SETTING_SEND_ENABLED.equals(setting)) {
                bluetoothSession.setSendEnabled(deviceSettings.sendEnabled);
            } else if (SETTING_RECEIVE_ENABLED.equals(setting)) {
                bluetoothSession.setReceiveEnabled(deviceSettings.receiveEnabled);
            } else if (SETTING_RECEIVE_VOLUME.equals(setting)) {
                bluetoothSession.setReceiveVolume(deviceSettings.receiveVolume);
            }
        } else {
            if (!lanSession.isConnected()) return;
            if (SETTING_SEND_ENABLED.equals(setting)) {
                lanSession.setSendEnabled(deviceSettings.sendEnabled);
            } else if (SETTING_RECEIVE_ENABLED.equals(setting)) {
                lanSession.setReceiveEnabled(deviceSettings.receiveEnabled);
            } else if (SETTING_RECEIVE_VOLUME.equals(setting)) {
                lanSession.setReceiveVolume(deviceSettings.receiveVolume);
            }
        }
    }

    private static String friendlyPeerName(String name, String fallback) {
        return name == null || name.trim().isEmpty() ? fallback : name.trim();
    }

    private String deviceDisplayName(String transport, String identity, String fallback) {
        return friendlyPeerName(settings().getString(
                devicePreferenceKey(transport, identity, "display_name"), null), fallback);
    }

    private void rememberDeviceName(String transport, String identity, String name) {
        if (identity.isEmpty() || name == null || name.trim().isEmpty()) return;
        settings().edit().putString(devicePreferenceKey(transport, identity, "display_name"),
                name.trim()).apply();
    }

    private static String bluetoothDeviceName(BluetoothDevice device) {
        try {
            return friendlyPeerName(device.getName(), "蓝牙设备");
        } catch (SecurityException error) {
            return "蓝牙设备";
        }
    }

    private static String bluetoothDeviceAddress(BluetoothDevice device) {
        try {
            return device.getAddress();
        } catch (SecurityException error) {
            return "";
        }
    }

    private boolean isLanListeningEnabled() {
        return settings().getBoolean(PREF_LAN_LISTEN_ENABLED, false);
    }

    private boolean isBluetoothListeningEnabled() {
        return settings().getBoolean(PREF_BLUETOOTH_LISTEN_ENABLED, false);
    }

    private void restoreConfiguredState(boolean publishStatus) {
        configuredStateRestored = true;
        boolean restore = settings().getBoolean(PREF_RESTORE_CONNECTION_STATE, true);
        boolean lanEnabled = restore && isLanListeningEnabled();
        boolean bluetoothEnabled = restore && isBluetoothListeningEnabled();
        if (!restore) {
            settings().edit()
                    .putBoolean(PREF_LAN_LISTEN_ENABLED, false)
                    .putBoolean(PREF_BLUETOOTH_LISTEN_ENABLED, false)
                    .apply();
        }
        if (lanEnabled) {
            try {
                lanSession.listen();
                if (publishStatus) publishStatus("局域网监听已启动");
            } catch (Exception error) {
                publishStatus("局域网监听启动失败：" + describe(error));
            }
        } else {
            lanSession.stopListening();
        }
        if (bluetoothEnabled) {
            try {
                bluetoothSession.listen();
                if (publishStatus) publishStatus("蓝牙监听已启动");
            } catch (Exception error) {
                publishStatus("蓝牙监听启动失败：" + describe(error));
            }
        } else {
            bluetoothSession.stopListening();
            bluetoothSession.close();
        }
        updateForegroundTypes(bluetoothEnabled);
        if (lanEnabled) {
            String host = settings().getString(PREF_LAN_HOST, "");
            int port = settings().getInt(PREF_LAN_PORT, LanSession.DEFAULT_PORT);
            if (host != null && !host.trim().isEmpty() && !lanSession.isConnected()) {
                bluetoothMode = false;
                if (isAutoConnectEnabled()) {
                    connectWithRetry(host, port, null);
                } else {
                    connect(host, port);
                }
            }
        }
        if (bluetoothEnabled && isAutoConnectEnabled() && hasBluetoothPermission()) {
            String address = settings().getString(PREF_BLUETOOTH_DEVICE_ADDRESS, "");
            if (address != null && !address.trim().isEmpty()) {
                try {
                    BluetoothDevice device = BluetoothAdapter.getDefaultAdapter()
                            .getRemoteDevice(address.trim());
                    bluetoothMode = true;
                    connectBluetoothWithRetry(device);
                } catch (RuntimeException ignored) {
                    publishStatus("无法恢复蓝牙设备连接");
                }
            }
        }
    }

    private void setLanListening(boolean enabled) {
        settings().edit().putBoolean(PREF_LAN_LISTEN_ENABLED, enabled).apply();
        if (enabled) {
            try {
                lanSession.listen();
                publishStatus("局域网监听已启动");
            } catch (Exception error) {
                publishStatus("局域网监听启动失败：" + describe(error));
            }
        } else {
            lanConnectionGeneration++;
            lanConnecting = false;
            lanSession.close();
            settings().edit().remove(PREF_LAN_HOST).remove(PREF_LAN_PORT).apply();
            if (!bluetoothSession.isConnected()) bluetoothMode = false;
            publishStatus("局域网监听已关闭");
        }
        updateTransferWakeLocks();
        updateForegroundTypes(isBluetoothListeningEnabled());
    }

    private void disconnectTransport(String transport) {
        if (TRANSPORT_LAN.equals(transport)) {
            lanSession.disconnect();
            lanConnecting = false;
            lanPeerName = "局域网设备";
            lanDeviceId = "";
            lanConnectedAtElapsedRealtime = 0;
            updateTransferWakeLocks();
            settings().edit().remove(PREF_LAN_HOST).remove(PREF_LAN_PORT).apply();
            publishStatus("已断开局域网设备");
            return;
        }
        if (TRANSPORT_BLUETOOTH.equals(transport)) {
            bluetoothConnectionGeneration++;
            bluetoothConnectionPending = false;
            bluetoothSession.disconnect();
            bluetoothMode = false;
            bluetoothConnectionPending = false;
            bluetoothConnectedAtElapsedRealtime = 0;
            bluetoothPeerAddress = "";
            bluetoothDeviceId = "";
            bluetoothPeerName = "蓝牙设备";
            updateTransferWakeLocks();
            publishStatus("已断开蓝牙设备");
            return;
        }
        publishStatus("未识别的连接类型");
    }

    private void setBluetoothListening(boolean enabled) {
        settings().edit().putBoolean(PREF_BLUETOOTH_LISTEN_ENABLED, enabled).apply();
        if (enabled) {
            try {
                bluetoothSession.listen();
                publishStatus("蓝牙监听已启动");
            } catch (Exception error) {
                publishStatus("蓝牙监听启动失败：" + describe(error));
            }
        } else {
            bluetoothConnectionGeneration++;
            bluetoothConnectionPending = false;
            bluetoothSession.stopListening();
            bluetoothSession.close();
            bluetoothMode = false;
            bluetoothConnectionPending = false;
            bluetoothConnectedAtElapsedRealtime = 0;
            bluetoothPeerAddress = "";
            bluetoothDeviceId = "";
            bluetoothPeerName = "蓝牙设备";
            publishStatus("蓝牙监听已关闭");
        }
        updateTransferWakeLocks();
        updateForegroundTypes(enabled);
    }

    private void updateForegroundTypes(boolean bluetoothEnabled) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            int types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK;
            if (bluetoothEnabled) types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
            startForeground(NOTIFICATION_ID, createNotification(), types);
        }
    }

    private void connect(String host, int port) {
        connect(host, port, null);
    }

    private void connect(String host, int port, String selectedName) {
        startLanConnection(host, port, selectedName, false);
    }

    private void connectWithRetry(String host, int port, String selectedName) {
        startLanConnection(host, port, selectedName, true);
    }

    private void startLanConnection(String host, int port, String selectedName, boolean retry) {
        if (host == null || host.trim().isEmpty()) {
            publishStatus("局域网设备地址无效");
            return;
        }
        if (lanConnecting) return;
        int generation = ++lanConnectionGeneration;
        lanConnecting = true;
        updateTransferWakeLocks();
        String normalizedHost = host.trim();
        lanDeviceId = normalizedHost.toLowerCase(java.util.Locale.ROOT) + ":" + port;
        rememberDeviceName(TRANSPORT_LAN, lanDeviceId, selectedName);
        lanPeerName = deviceDisplayName(TRANSPORT_LAN, lanDeviceId, "局域网设备");
        lanAudioSettings = loadAudioSettings(TRANSPORT_LAN, lanDeviceId);
        settings().edit()
                .putString(PREF_LAN_HOST, normalizedHost)
                .putInt(PREF_LAN_PORT, port)
                .apply();
        publishStatus("正在连接局域网设备：" + lanPeerName);
        controlExecutor.execute(() -> {
            int attempts = 0;
            while (generation == lanConnectionGeneration && isLanListeningEnabled()) {
                attempts++;
                try {
                    lanSession.listen();
                    lanSession.connect(normalizedHost, port);
                    lanSession.awaitHandshake(5000);
                    publishStatus("局域网连接已就绪");
                    break;
                } catch (Exception error) {
                    publishStatus("局域网连接失败：" + describe(error));
                    if (!retry || !isAutoConnectEnabled()
                            || (autoConnectAttempts() > 0 && attempts >= autoConnectAttempts())) {
                        break;
                    }
                    if (!sleepBeforeRetry(generation, autoConnectIntervalSeconds())) break;
                }
            }
            if (generation == lanConnectionGeneration) {
                lanConnecting = false;
                updateTransferWakeLocks();
            }
        });
    }

    private void connectBluetoothWithRetry(BluetoothDevice device) {
        if (device == null || bluetoothConnectionPending) return;
        int generation = ++bluetoothConnectionGeneration;
        bluetoothConnectionPending = true;
        updateTransferWakeLocks();
        controlExecutor.execute(() -> {
            int attempts = 0;
            while (generation == bluetoothConnectionGeneration && isBluetoothListeningEnabled()) {
                attempts++;
                bluetoothSession.setInitialSendEnabled(bluetoothAudioSettings.sendEnabled);
                bluetoothSession.connect(device, bluetoothAudioSettings.sendEnabled);
                long deadline = System.currentTimeMillis() + autoConnectIntervalSeconds() * 1000L;
                while (generation == bluetoothConnectionGeneration
                        && !bluetoothSession.isConnected()
                        && System.currentTimeMillis() < deadline) {
                    if (!sleepBeforeRetry(generation, 0)) break;
                }
                if (bluetoothSession.isConnected()) break;
                if (!isAutoConnectEnabled()
                        || (autoConnectAttempts() > 0 && attempts >= autoConnectAttempts())) break;
            }
            if (generation == bluetoothConnectionGeneration) {
                bluetoothConnectionPending = false;
                updateTransferWakeLocks();
            }
        });
    }

    private boolean sleepBeforeRetry(int generation, int seconds) {
        long duration = seconds > 0 ? seconds * 1000L : 100L;
        try {
            Thread.sleep(duration);
            return generation == lanConnectionGeneration
                    || generation == bluetoothConnectionGeneration;
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    private boolean isAutoConnectEnabled() {
        return settings().getBoolean(PREF_AUTO_CONNECT, true);
    }

    private int autoConnectAttempts() {
        return Math.max(0, Math.min(5, settings().getInt(PREF_AUTO_CONNECT_ATTEMPTS, 5)));
    }

    private int autoConnectIntervalSeconds() {
        int value = settings().getInt(PREF_AUTO_CONNECT_INTERVAL_SECONDS, 5);
        return value == 1 || value == 2 || value == 3 || value == 5 || value == 10 || value == 30
                ? value : 5;
    }

    private boolean hasBluetoothPermission() {
        return Build.VERSION.SDK_INT < 31
                || checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT)
                == PackageManager.PERMISSION_GRANTED;
    }

    private void startCapture(Intent intent) {
        if (capturing) {
            publishStatus("手机音频录制已在运行");
            return;
        }
        Thread previousCaptureThread = captureThread;
        if (previousCaptureThread != null && previousCaptureThread.isAlive()) {
            publishStatus("正在停止上一条音频录制任务，请稍后重试");
            return;
        }
        int resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, 0);
        Intent resultData = getResultData(intent);
        if (resultCode == 0 || resultData == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            int foregroundTypes = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
                    | ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK;
            if (isBluetoothListeningEnabled() || bluetoothSession.isConnected()) {
                foregroundTypes |= ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
            }
            startForeground(NOTIFICATION_ID, createNotification(),
                    foregroundTypes);
        }
        MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
        mediaProjection = manager.getMediaProjection(resultCode, resultData);
        if (mediaProjection == null) {
            publishStatus("系统播放捕获授权无效");
            return;
        }
        AudioFormat format = new AudioFormat.Builder()
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(CAPTURE_SAMPLE_RATE)
                .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                .build();
        AudioAttributes attributes = new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build();
        try {
            AudioRecord.Builder builder = new AudioRecord.Builder()
                    .setAudioFormat(format);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                builder.setAudioPlaybackCaptureConfig(
                        new android.media.AudioPlaybackCaptureConfiguration.Builder(mediaProjection)
                                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                                .build());
            }
            int minBuffer = AudioRecord.getMinBufferSize(
                    CAPTURE_SAMPLE_RATE, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT);
            if (minBuffer <= 0) {
                throw new IllegalStateException("设备不支持 48 kHz 双声道系统音频采集");
            }
            audioRecord = builder.setBufferSizeInBytes(Math.max(minBuffer, CAPTURE_FRAME_BYTES * 2)).build();
            if (audioRecord.getState() != AudioRecord.STATE_INITIALIZED) {
                throw new IllegalStateException("双声道 AudioRecord 初始化失败");
            }
            if (audioRecord.getChannelCount() != CAPTURE_CHANNELS) {
                throw new IllegalStateException("系统未提供双声道音频采集，无法保留左右声道");
            }
            audioRecord.startRecording();
            if (audioRecord.getRecordingState() != AudioRecord.RECORDSTATE_RECORDING) {
                throw new IllegalStateException("AudioRecord 未进入录音状态");
            }
            capturing = true;
            updateTransferWakeLocks();
            capturedFrames = 0;
            sentFrames = 0;
            silentFrames = 0;
            publishStatus("手机双声道音频录制已启动");
            AudioRecord activeRecorder = audioRecord;
            captureThread = new Thread(() -> captureLoop(activeRecorder), "AudioRouter-capture");
            captureThread.start();
        } catch (SecurityException | IllegalArgumentException | IllegalStateException error) {
            publishStatus("系统播放捕获启动失败：" + describe(error));
            stopCapture();
        }
    }

    private void captureLoop(AudioRecord activeRecorder) {
        try {
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO);
        } catch (RuntimeException ignored) {
        }
        byte[] buffer = new byte[CAPTURE_FRAME_BYTES];
        long lastStatusAt = 0;
        while (capturing && audioRecord == activeRecorder) {
            int count = activeRecorder.read(buffer, 0, buffer.length, AudioRecord.READ_BLOCKING);
            if (count <= 0) {
                if (!capturing || audioRecord != activeRecorder) break;
                continue;
            }
            if (!capturing || audioRecord != activeRecorder) break;
            capturedFrames++;
            if (isSilent(buffer, count)) {
                silentFrames++;
            }
            int usableCount = count - count % CAPTURE_BLOCK_BYTES;
            if (usableCount == 0) {
                continue;
            }
            byte[] frame = Arrays.copyOf(buffer, usableCount);
            try {
                sentFrames += sendAudioToEnabledDevices(frame, CAPTURE_SAMPLE_RATE, CAPTURE_CHANNELS);
                lastStatusAt = publishCaptureStatusIfDue(lastStatusAt,
                        "手机音频录制进行中");
            } catch (Exception error) {
                publishStatus("音频发送失败：" + describe(error));
                capturing = false;
                break;
            }
        }
        if (audioRecord == activeRecorder) {
            capturing = false;
            updateTransferWakeLocks();
            publishStatus("手机音频录制已停止");
        }
    }

    private long publishCaptureStatusIfDue(long lastStatusAt, String message) {
        long now = System.currentTimeMillis();
        if (now - lastStatusAt < 1000) {
            return lastStatusAt;
        }
        publishStatus(message + "（捕获 " + capturedFrames + " 帧，已发送 "
                + sentFrames + " 路音频帧）");
        return now;
    }

    private static boolean isSilent(byte[] pcm, int length) {
        for (int index = 0; index + 1 < length; index += 2) {
            if (pcm[index] != 0 || pcm[index + 1] != 0) {
                return false;
            }
        }
        return true;
    }

    private float effectiveSendGain(DeviceAudioSettings deviceSettings) {
        float manualGain = PcmAudio.volumeToGain(deviceSettings.sendVolume);
        if (!deviceSettings.followSystemVolume) return manualGain;
        AudioManager manager = audioManager;
        if (manager == null) return manualGain;
        int maximum = manager.getStreamMaxVolume(AudioManager.STREAM_MUSIC);
        int current = manager.getStreamVolume(AudioManager.STREAM_MUSIC);
        if (maximum <= 0 || manager.isStreamMute(AudioManager.STREAM_MUSIC)) return 0.0f;
        // AudioManager already exposes the system mix as a user-facing scalar.
        // Applying the dB slider curve to it again would attenuate normal levels twice.
        float systemGain = Math.max(0.0f, Math.min(1.0f, current / (float) maximum));
        return manualGain * systemGain;
    }

    private static String describe(Throwable error) {
        String message = error.getMessage();
        return message == null || message.trim().isEmpty()
                ? error.getClass().getSimpleName()
                : message;
    }

    private void sendControl(ControlAction action) {
        controlExecutor.execute(() -> {
            try {
                action.run();
            } catch (Exception error) {
                publishStatus("控制消息发送失败：" + describe(error));
            }
        });
    }

    private void publishStatus(String status) {
        boolean lanConnected = lanSession != null && lanSession.isConnected();
        boolean bluetoothConnected = bluetoothSession != null && bluetoothSession.isConnected();
        boolean connected = lanConnected || bluetoothConnected;
        sendBroadcast(new Intent(ACTION_STATUS)
                .setPackage(getPackageName())
                .putExtra(EXTRA_STATUS, status)
                .putExtra(EXTRA_CAPTURING, capturing)
                .putExtra(EXTRA_CONNECTED, connected)
                .putExtra(EXTRA_SEND_ENABLED, lanAudioSettings.sendEnabled
                        || bluetoothAudioSettings.sendEnabled)
                .putExtra(EXTRA_REMOTE_SEND_ENABLED,
                        (lanConnected && lanSession.isRemoteSendEnabled())
                                || (bluetoothConnected && bluetoothSession.isRemoteSendEnabled()))
                .putExtra(EXTRA_LAN_CONNECTED, lanConnected)
                .putExtra(EXTRA_LAN_LISTEN_ENABLED, isLanListeningEnabled())
                .putExtra(EXTRA_LAN_NAME, lanPeerName)
                .putExtra(EXTRA_LAN_DEVICE_ID, lanDeviceId)
                .putExtra(EXTRA_LAN_CONNECTED_AT,
                        lanConnected ? lanConnectedAtElapsedRealtime : 0)
                .putExtra(EXTRA_BLUETOOTH_CONNECTED, bluetoothConnected)
                .putExtra(EXTRA_BLUETOOTH_LISTEN_ENABLED, isBluetoothListeningEnabled())
                .putExtra(EXTRA_BLUETOOTH_NAME, bluetoothPeerName)
                .putExtra(EXTRA_BLUETOOTH_ADDRESS, bluetoothPeerAddress)
                .putExtra(EXTRA_BLUETOOTH_DEVICE_ID, bluetoothDeviceId)
                .putExtra(EXTRA_BLUETOOTH_CONNECTED_AT,
                        bluetoothConnected ? bluetoothConnectedAtElapsedRealtime : 0));
    }

    private int sendAudioToEnabledDevices(byte[] frame, int sampleRate, int channels) {
        int destinations = 0;
        if (lanSession.isConnected() && lanAudioSettings.sendEnabled
                && lanSession.isRemoteSendEnabled()) {
            try {
                byte[] lanFrame = PcmAudio.applyGainCopy(frame, effectiveSendGain(lanAudioSettings));
                int lanChannels = channels;
                if (AudioQuality.LAN_UNCOMPRESSED.equals(lanCodecMode)
                        && lanPcmChannels != AudioQuality.DEFAULT_PCM_CHANNELS) {
                    lanFrame = PcmAudio.convertChannels(lanFrame, channels, lanPcmChannels);
                    lanChannels = lanPcmChannels;
                }
                lanSession.sendAudio(lanFrame, sampleRate, lanChannels);
                destinations++;
            } catch (Exception error) {
                publishStatus("局域网音频发送失败：" + describe(error));
            }
        }
        if (bluetoothSession.isConnected() && bluetoothAudioSettings.sendEnabled
                && bluetoothSession.isRemoteSendEnabled()) {
            try {
                bluetoothSession.sendAudio(
                        PcmAudio.applyGainCopy(frame, effectiveSendGain(bluetoothAudioSettings)),
                        sampleRate, channels);
                destinations++;
            } catch (Exception error) {
                publishStatus("蓝牙音频发送失败：" + describe(error));
            }
        }
        return destinations;
    }

    private void stopCapture() {
        capturing = false;
        AudioRecord recorder = audioRecord;
        audioRecord = null;
        if (recorder != null) {
            try {
                recorder.stop();
            } catch (IllegalStateException ignored) {
            }
            recorder.release();
        }
        MediaProjection projection = mediaProjection;
        mediaProjection = null;
        if (projection != null) {
            projection.stop();
        }
        Thread worker = captureThread;
        captureThread = null;
        if (worker != null && worker != Thread.currentThread()) {
            try {
                worker.join(1000);
            } catch (InterruptedException error) {
                Thread.currentThread().interrupt();
            }
        }
        updateTransferWakeLocks();
    }

    private void updateTransferWakeLocks() {
        boolean keepCpuAwake = capturing || lanConnecting || bluetoothConnectionPending
                || (lanSession != null && lanSession.isConnected())
                || (bluetoothSession != null && bluetoothSession.isConnected());
        if (transferWakeLock != null) {
            if (keepCpuAwake && !transferWakeLock.isHeld()) {
                transferWakeLock.acquire();
            } else if (!keepCpuAwake && transferWakeLock.isHeld()) {
                transferWakeLock.release();
            }
        }
        boolean keepWifiAwake = lanConnecting || (lanSession != null && lanSession.isConnected());
        if (lanWifiLock != null) {
            if (keepWifiAwake && !lanWifiLock.isHeld()) {
                lanWifiLock.acquire();
            } else if (!keepWifiAwake && lanWifiLock.isHeld()) {
                lanWifiLock.release();
            }
        }
    }

    private void releaseTransferWakeLocks() {
        if (transferWakeLock != null && transferWakeLock.isHeld()) {
            transferWakeLock.release();
        }
        if (lanWifiLock != null && lanWifiLock.isHeld()) {
            lanWifiLock.release();
        }
    }

    @SuppressWarnings("deprecation")
    private Intent getResultData(Intent intent) {
        if (Build.VERSION.SDK_INT >= 33) {
            return intent.getParcelableExtra(EXTRA_RESULT_DATA, Intent.class);
        }
        return intent.getParcelableExtra(EXTRA_RESULT_DATA);
    }

    @SuppressWarnings("deprecation")
    private BluetoothDevice getBluetoothDevice(Intent intent) {
        if (Build.VERSION.SDK_INT >= 33) {
            return intent.getParcelableExtra(EXTRA_DEVICE, BluetoothDevice.class);
        }
        return intent.getParcelableExtra(EXTRA_DEVICE);
    }

    private static int clamp(int value) {
        return Math.max(0, Math.min(100, value));
    }

    @Override
    public void onDestroy() {
        stopCapture();
        controlExecutor.shutdownNow();
        if (lanSession != null) {
            lanSession.close();
        }
        if (bluetoothSession != null) {
            bluetoothSession.shutdown();
        }
        releaseTransferWakeLocks();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private Notification createNotification() {
        return new Notification.Builder(this, CHANNEL_ID)
                .setContentTitle("AudioRouter")
                .setContentText("局域网音频服务运行中")
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setOngoing(true)
                .build();
    }

    private void createNotificationChannel() {
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "AudioRouter 音频服务", NotificationManager.IMPORTANCE_LOW);
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
    }

    private interface ControlAction {
        void run() throws Exception;
    }

    private static final class DeviceAudioSettings {
        volatile boolean sendEnabled = true;
        volatile int sendVolume = 100;
        volatile boolean followSystemVolume;
        volatile boolean receiveEnabled = true;
        volatile int receiveVolume = 100;
    }
}
