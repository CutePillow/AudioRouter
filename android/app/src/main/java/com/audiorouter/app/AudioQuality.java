package com.audiorouter.app;

final class AudioQuality {
    static final String LAN_COMPRESSED = "compressed";
    static final String LAN_UNCOMPRESSED = "uncompressed";
    static final String BLUETOOTH_AUTO = "auto";
    static final String BLUETOOTH_OPUS = "opus";
    static final String BLUETOOTH_PCM = "pcm";

    static final int DEFAULT_LAN_OPUS_BITRATE_KBPS = 128;
    static final int DEFAULT_BLUETOOTH_OPUS_BITRATE_KBPS = 80;
    static final int DEFAULT_PCM_CHANNELS = 0;

    static final int CAPABILITY_PCM = 0x01;
    static final int CAPABILITY_OPUS = 0x04;
    static final int CAPABILITY_PREFERENCE_SHIFT = 8;
    static final int CAPABILITY_BITRATE_SHIFT = 10;

    static final int PREFERENCE_AUTO = 0;
    static final int PREFERENCE_OPUS = 1;
    static final int PREFERENCE_PCM = 2;

    private AudioQuality() {
    }

    static int buildLanCapabilities(String mode, int bitrateKbps) {
        int preference = LAN_UNCOMPRESSED.equals(mode) ? PREFERENCE_PCM : PREFERENCE_OPUS;
        return buildCapabilities(preference, bitrateKbps);
    }

    static int buildBluetoothCapabilities(String mode) {
        int preference;
        if (BLUETOOTH_OPUS.equals(mode)) {
            preference = PREFERENCE_OPUS;
        } else if (BLUETOOTH_PCM.equals(mode)) {
            preference = PREFERENCE_PCM;
        } else {
            preference = PREFERENCE_AUTO;
        }
        return buildCapabilities(preference, DEFAULT_BLUETOOTH_OPUS_BITRATE_KBPS);
    }

    static int buildCapabilities(int preference, int bitrateKbps) {
        int safePreference = preference < PREFERENCE_AUTO || preference > PREFERENCE_PCM
                ? PREFERENCE_AUTO : preference;
        int safeBitrate = Math.max(6, Math.min(450, bitrateKbps));
        return CAPABILITY_PCM | CAPABILITY_OPUS
                | (safePreference << CAPABILITY_PREFERENCE_SHIFT)
                | (safeBitrate << CAPABILITY_BITRATE_SHIFT);
    }

    static boolean supportsOpus(int capabilities) {
        return (capabilities & CAPABILITY_OPUS) != 0;
    }

}
