# Windows 端

Windows 端位于 `crates/audiorouter-desktop`，负责 WASAPI Loopback 捕获、音频播放、局域网服务、Bluetooth Classic 服务以及桌面窗口和托盘行为。

界面源文件位于 `assets/ui/pc-ui-mock.html`，构建时会嵌入程序，不依赖 `Self-disk` 中的本地副本。

程序运行期间的设置和 WebView2 数据使用程序目录下的 `data` 文件夹，以便安装包支持自定义安装位置并保持数据位置一致。
