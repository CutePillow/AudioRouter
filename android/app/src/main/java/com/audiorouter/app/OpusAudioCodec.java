package com.audiorouter.app;

import org.concentus.OpusApplication;
import org.concentus.OpusDecoder;
import org.concentus.OpusEncoder;
import org.concentus.OpusException;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

final class OpusAudioCodec {
    static final int SAMPLE_RATE = 48_000;
    static final int FRAME_SAMPLES_PER_CHANNEL = 960;
    private static final int MAX_PACKET_BYTES = 4_000;
    private static final int MAX_DECODE_SAMPLES_PER_CHANNEL = 5_760;

    static final class Encoder {
        private final int channels;
        private final OpusEncoder encoder;
        private final byte[] pending;
        private final short[] samples;
        private final byte[] encoded = new byte[MAX_PACKET_BYTES];
        private int pendingBytes;

        Encoder(int channels) throws IOException {
            this(channels, channels == 1 ? 32 : 80);
        }

        Encoder(int channels, int bitrateKbps) throws IOException {
            validateChannels(channels);
            this.channels = channels;
            samples = new short[FRAME_SAMPLES_PER_CHANNEL * channels];
            pending = new byte[samples.length * 2];
            try {
                encoder = new OpusEncoder(SAMPLE_RATE, channels, OpusApplication.OPUS_APPLICATION_AUDIO);
                encoder.setBitrate(Math.max(6, Math.min(450, bitrateKbps)) * 1_000);
                encoder.setComplexity(4);
            } catch (OpusException error) {
                throw new IOException("创建 Opus 编码器失败", error);
            }
        }

        int channels() {
            return channels;
        }

        List<byte[]> encode(byte[] pcm16le) throws IOException {
            if ((pcm16le.length & 1) != 0) {
                throw new IOException("PCM16LE 数据长度必须为偶数");
            }
            int frameBytes = pending.length;
            List<byte[]> packets = new ArrayList<>((pendingBytes + pcm16le.length) / frameBytes);
            int inputOffset = 0;
            if (pendingBytes > 0) {
                int copied = Math.min(frameBytes - pendingBytes, pcm16le.length);
                System.arraycopy(pcm16le, 0, pending, pendingBytes, copied);
                pendingBytes += copied;
                inputOffset += copied;
                if (pendingBytes < frameBytes) return packets;
                pendingBytes = 0;
                packets.add(encodeFrame(pending, 0));
            }
            while (pcm16le.length - inputOffset >= frameBytes) {
                packets.add(encodeFrame(pcm16le, inputOffset));
                inputOffset += frameBytes;
            }
            pendingBytes = pcm16le.length - inputOffset;
            System.arraycopy(pcm16le, inputOffset, pending, 0, pendingBytes);
            return packets;
        }

        private byte[] encodeFrame(byte[] pcm, int offset) throws IOException {
            for (int sampleIndex = 0; sampleIndex < samples.length; sampleIndex++) {
                samples[sampleIndex] = PcmAudio.readSample(pcm, offset + sampleIndex * 2);
            }
            int encodedBytes;
            try {
                encodedBytes = encoder.encode(samples, 0, FRAME_SAMPLES_PER_CHANNEL,
                        encoded, 0, encoded.length);
            } catch (OpusException error) {
                throw new IOException("Opus 编码失败", error);
            }
            if (encodedBytes <= 0 || encodedBytes > encoded.length) {
                throw new IOException("Opus 编码器返回无效数据长度：" + encodedBytes);
            }
            return Arrays.copyOf(encoded, encodedBytes);
        }
    }

    static final class Decoder {
        private final int channels;
        private final OpusDecoder decoder;
        private final short[] samples;

        Decoder(int channels) throws IOException {
            validateChannels(channels);
            this.channels = channels;
            samples = new short[MAX_DECODE_SAMPLES_PER_CHANNEL * channels];
            try {
                decoder = new OpusDecoder(SAMPLE_RATE, channels);
            } catch (OpusException error) {
                throw new IOException("创建 Opus 解码器失败", error);
            }
        }

        byte[] decode(byte[] packet) throws IOException {
            if (packet.length == 0) {
                throw new IOException("Opus 音频包不能为空");
            }
            int samplesPerChannel;
            try {
                samplesPerChannel = decoder.decode(packet, 0, packet.length, samples, 0,
                        MAX_DECODE_SAMPLES_PER_CHANNEL, false);
            } catch (OpusException error) {
                throw new IOException("Opus 解码失败", error);
            }
            if (samplesPerChannel <= 0 || samplesPerChannel > MAX_DECODE_SAMPLES_PER_CHANNEL) {
                throw new IOException("Opus 解码器返回无效采样数：" + samplesPerChannel);
            }
            byte[] pcm = new byte[samplesPerChannel * channels * 2];
            for (int sampleIndex = 0; sampleIndex < samplesPerChannel * channels; sampleIndex++) {
                PcmAudio.writeSample(pcm, sampleIndex * 2, samples[sampleIndex]);
            }
            return pcm;
        }
    }

    private static void validateChannels(int channels) throws IOException {
        if (channels != 1 && channels != 2) {
            throw new IOException("Opus 只支持单声道或双声道，收到 " + channels);
        }
    }

    private OpusAudioCodec() {
    }
}
