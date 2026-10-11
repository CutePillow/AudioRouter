# AudioRouter 第一版开发计划

## 目标

先完成 Android 16 与 Windows 之间的局域网播放音频传输，再将同一协议适配到 Bluetooth Classic。

## 当前完成项

- 建立 Rust workspace
- 建立共享核心 crate
- 定义 v1 音频包和控制消息格式
- 实现音频包、握手、开关和音量编码/解码
- 增加协议错误校验、握手集成测试
- 接入 Android 16 `AudioPlaybackCapture` 和 `AudioTrack`
- 接入 Windows 局域网主机和 PCM 播放输出
- 接入 Windows WASAPI Loopback 播放音频捕获
- 实现 Windows → Android PCM 音频广播
- 增加局域网握手和广播集成测试

## 下一步

1. 增加多设备连接和每个远端设备独立音量。
2. 接入 Opus 编码器和抖动缓冲，降低局域网和蓝牙带宽。
3. 实现 Bluetooth Classic Transport 适配器。
4. 做真实 Windows/Android 设备端到端试听和长时间稳定性测试。

## 当前第一版验收边界

- Windows 主机监听 TCP `40400`，Android 16 客户端可以连接。
- 只传输播放音频。
- Android 播放音频经过用户授权后可以发送到 Windows。
- Windows 可以接收 PCM 音频并输出到默认播放设备。
- Windows 播放音频经过 WASAPI Loopback 捕获后可以发送到 Android。
- 发送和接收可以独立关闭。
- 发送音量和接收音量可以独立调整。
- 可以选择是否恢复上次关闭时的连接方式，并配置自动连接尝试次数和间隔。
