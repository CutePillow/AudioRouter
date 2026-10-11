package com.audiorouter.app;

final class PcmAudio {
    private PcmAudio() {
    }

    static float volumeToGain(int volume) {
        int clamped = Math.max(0, Math.min(100, volume));
        return clamped / 100.0f;
    }

    static byte[] applyGainCopy(byte[] pcm, float gain) {
        return gain >= 0.999999f ? pcm : scaleCopy(pcm, gain);
    }

    static byte[] applyPlaybackGain(byte[] pcm, float gain) {
        return Math.abs(gain - 1.0f) < 0.001f ? pcm : scaleCopy(pcm, gain);
    }

    private static byte[] scaleCopy(byte[] pcm, float gain) {
        byte[] output = pcm.clone();
        for (int offset = 0; offset + 1 < output.length; offset += 2) {
            int scaled = Math.round(readSample(output, offset) * gain);
            writeSample(output, offset, (short) Math.max(Short.MIN_VALUE, Math.min(Short.MAX_VALUE, scaled)));
        }
        return output;
    }

    static byte[] convertChannels(byte[] pcm, int sourceChannels, int targetChannels) {
        if (sourceChannels == targetChannels || (sourceChannels != 1 && sourceChannels != 2)
                || (targetChannels != 1 && targetChannels != 2)) {
            return pcm;
        }
        int sourceFrameBytes = sourceChannels * 2;
        int frameCount = pcm.length / sourceFrameBytes;
        byte[] output = new byte[frameCount * targetChannels * 2];
        for (int frame = 0; frame < frameCount; frame++) {
            int sourceOffset = frame * sourceFrameBytes;
            short first = readSample(pcm, sourceOffset);
            short second = sourceChannels == 2 ? readSample(pcm, sourceOffset + 2) : first;
            if (targetChannels == 1) {
                writeSample(output, frame * 2, (short) ((first + second) / 2));
            } else {
                int outputOffset = frame * 4;
                writeSample(output, outputOffset, first);
                writeSample(output, outputOffset + 2, second);
            }
        }
        return output;
    }

    static short readSample(byte[] pcm, int offset) {
        return (short) ((pcm[offset] & 0xff) | (pcm[offset + 1] << 8));
    }

    static void writeSample(byte[] pcm, int offset, short sample) {
        pcm[offset] = (byte) sample;
        pcm[offset + 1] = (byte) (sample >> 8);
    }
}
