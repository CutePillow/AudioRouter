# 构建说明

## Rust 与 Windows 端

```text
cargo test --workspace --offline
cargo build -p audiorouter-desktop --release --offline
```

构建产物位于 `target/release`。Windows 端需要 WebView2 Runtime 才能正常显示界面。
项目内的 `.cargo/config.toml` 已为 Opus 依赖配置新版 CMake 所需的兼容开关。

## Android 端

使用 JDK 17、Android SDK API 36 和 Gradle 8.9：

```text
tools/build-android.ps1
```

脚本会将 APK 复制到本地 `Self-disk` 目录。

## Windows 安装包

需要本机已有 Inno Setup 7：

```text
tools/build-installer.ps1 -Version 1.0.0
```

安装器默认安装到 `C:\Program Files\AudioRouter`，也允许用户选择其他目录。程序配置、设备偏好、窗口大小和 WebView2 数据保存在安装目录的 `data` 子目录中。

## 本地交付目录

`Self-disk` 只用于保存本地 APK、EXE、安装包、UI 预览和运行数据，已加入 `.gitignore`。发布 GitHub 时建议将正式构建产物放入 GitHub Releases。
