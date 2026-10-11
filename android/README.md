# Android 端

Android 端负责捕获系统播放音频、通过局域网或 Bluetooth Classic 发送音频，并使用 `AudioTrack` 播放接收到的音频。

## 主要实现

- `AudioPlaybackCapture`：系统播放音频捕获。
- `LanSession`：局域网 TCP 音频会话。
- `BluetoothSession`：Bluetooth Classic RFCOMM 音频会话。
- `OpusAudioCodec` 与 `PcmAudio`：音频编码、解码和播放。
- `MainActivity`：原生 UI、设置页面和设备控制。

网页原型位于 `assets/ui/pc-ui-mock.html`，仅作为交互与视觉参考，Android 端不嵌入模拟网页。

## 构建

需要 JDK 17、Android SDK API 36 和 Gradle 8.9。推荐从项目根目录执行：

```text
tools/build-android.ps1
```

APK 会输出到本地 `Self-disk` 目录。该目录已加入 Git 忽略，不会上传到 GitHub。
