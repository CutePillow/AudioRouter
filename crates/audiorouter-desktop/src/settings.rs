use std::collections::HashMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use audiorouter_core::{PREFERENCE_AUTO, PREFERENCE_OPUS, PREFERENCE_PCM};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppTheme {
    Light,
    Dark,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DeviceSettings {
    pub send_enabled: bool,
    pub receive_enabled: bool,
    pub send_volume: u8,
    pub receive_volume: u8,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            send_enabled: false,
            receive_enabled: true,
            send_volume: 80,
            receive_volume: 80,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DesktopSettings {
    pub theme: AppTheme,
    pub window_width: u32,
    pub window_height: u32,
    pub preconnect_controls: bool,
    pub show_ip: bool,
    pub lan_compressed: bool,
    pub lan_bitrate_kbps: u32,
    pub bluetooth_preference: u8,
    pub buffer_ms: u16,
    pub custom_buffer_ms: u16,
    pub launch_on_startup: bool,
    pub exit_behavior: String,
    pub lan_enabled: bool,
    pub bluetooth_enabled: bool,
    pub restore_connection_state: bool,
    pub auto_connect: bool,
    pub auto_connect_attempts: u8,
    pub auto_connect_interval_seconds: u32,
    pub auto_connect_targets: Vec<String>,
    pub device_preferences: HashMap<String, DeviceSettings>,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            theme: AppTheme::System,
            window_width: 1180,
            window_height: 820,
            preconnect_controls: false,
            show_ip: false,
            lan_compressed: false,
            lan_bitrate_kbps: 128,
            bluetooth_preference: PREFERENCE_AUTO,
            buffer_ms: 60,
            custom_buffer_ms: 80,
            launch_on_startup: false,
            exit_behavior: "full_exit".to_string(),
            lan_enabled: false,
            bluetooth_enabled: false,
            restore_connection_state: false,
            auto_connect: false,
            auto_connect_attempts: 3,
            auto_connect_interval_seconds: 3,
            auto_connect_targets: Vec::new(),
            device_preferences: HashMap::new(),
        }
    }
}

impl DesktopSettings {
    pub fn load() -> Self {
        let Ok(path) = settings_file_path() else {
            return Self::default();
        };
        if path.exists() {
            return load_settings_from(&path);
        }
        if let Ok(legacy_path) = legacy_settings_file_path() {
            if legacy_path.exists() {
                let settings = load_settings_from(&legacy_path);
                let _ = save_settings_to(&path, &settings);
                return settings;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = settings_file_path()?;
        save_settings_to(&path, self)
    }

    pub fn data_directory() -> Result<PathBuf, String> {
        settings_file_path()?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "程序数据目录无效".to_string())
    }

    pub fn apply_startup_registration(enabled: bool) -> Result<(), String> {
        #[cfg(windows)]
        {
            update_windows_startup_registration(enabled)?;
        }
        Ok(())
    }

    fn normalize(&mut self) {
        if !(640..=3840).contains(&self.window_width) {
            self.window_width = 1180;
        }
        if !(480..=2160).contains(&self.window_height) {
            self.window_height = 820;
        }
        if !(32..=450).contains(&self.lan_bitrate_kbps) {
            self.lan_bitrate_kbps = 128;
        }
        if self.bluetooth_preference != PREFERENCE_AUTO
            && self.bluetooth_preference != PREFERENCE_OPUS
            && self.bluetooth_preference != PREFERENCE_PCM
        {
            self.bluetooth_preference = PREFERENCE_AUTO;
        }
        if self.buffer_ms != 0 && !(20..=240).contains(&self.buffer_ms) {
            self.buffer_ms = 60;
        }
        self.custom_buffer_ms = self.custom_buffer_ms.clamp(20, 240);
        if self.auto_connect_attempts > 5 {
            self.auto_connect_attempts = 3;
        }
        if ![1, 2, 3, 5, 10, 30].contains(&self.auto_connect_interval_seconds) {
            self.auto_connect_interval_seconds = 3;
        }
        if self.exit_behavior != "full_exit" && self.exit_behavior != "minimize_to_tray" {
            self.exit_behavior = "full_exit".to_string();
        }
        for device in self.device_preferences.values_mut() {
            device.send_volume = device.send_volume.min(100);
            device.receive_volume = device.receive_volume.min(100);
        }
    }
}

#[cfg(windows)]
fn update_windows_startup_registration(enabled: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect();
    let value_name: Vec<u16> = "AudioRouter\0".encode_utf16().collect();

    if enabled {
        let executable =
            env::current_exe().map_err(|error| format!("获取程序路径失败：{error}"))?;
        let command = format!("\"{}\"", executable.to_string_lossy());
        let command: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let mut key = std::ptr::null_mut();
        let mut disposition = 0;
        let create_status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                &mut disposition,
            )
        };
        if create_status != 0 {
            return Err(format!(
                "更新开机自启动设置失败：{}",
                std::io::Error::from_raw_os_error(create_status as i32)
            ));
        }
        let status = unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                ((command.len() - 1) * std::mem::size_of::<u16>()) as u32,
            )
        };
        unsafe {
            RegCloseKey(key);
        }
        if status != 0 {
            return Err(format!(
                "更新开机自启动设置失败：{}",
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
    } else {
        let mut key = std::ptr::null_mut();
        let open_status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if open_status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if open_status != 0 {
            return Err(format!(
                "更新开机自启动设置失败：{}",
                std::io::Error::from_raw_os_error(open_status as i32)
            ));
        }
        let status = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };
        unsafe {
            RegCloseKey(key);
        }
        if status != 0 && status != ERROR_FILE_NOT_FOUND {
            return Err(format!(
                "更新开机自启动设置失败：{}",
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
    }

    Ok(())
}

fn settings_file_path() -> Result<PathBuf, String> {
    let executable = env::current_exe().map_err(|error| format!("获取程序路径失败：{error}"))?;
    let executable_directory = executable
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .ok_or_else(|| "找不到程序安装目录，设置无法保存".to_string())?;

    Ok(executable_directory.join("data").join("settings.json"))
}

fn legacy_settings_file_path() -> Result<PathBuf, String> {
    let local_app_data = env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(|profile| PathBuf::from(profile).join("AppData").join("Local"))
        })
        .ok_or_else(|| "找不到本地应用数据目录，设置无法保存".to_string())?;

    Ok(local_app_data.join("AudioRouter").join("settings.json"))
}

fn load_settings_from(path: &Path) -> DesktopSettings {
    let Ok(contents) = fs::read(path) else {
        return DesktopSettings::default();
    };
    let Ok(mut settings) = serde_json::from_slice::<DesktopSettings>(&contents) else {
        return DesktopSettings::default();
    };
    settings.normalize();
    settings
}

fn save_settings_to(path: &Path, settings: &DesktopSettings) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "设置文件路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建设置目录失败：{error}"))?;

    let file_name = path
        .file_name()
        .ok_or_else(|| "设置文件路径无效".to_string())?
        .to_string_lossy();
    let temporary_path = path.with_file_name(format!("{file_name}.{}.tmp", std::process::id()));
    let mut normalized = settings.clone();
    normalized.normalize();
    let contents = serde_json::to_vec_pretty(&normalized)
        .map_err(|error| format!("生成设置文件失败：{error}"))?;

    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary_path)?;
        file.write_all(&contents)?;
        file.sync_all()?;
        replace_file(&temporary_path, path)
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("保存设置失败：{error}"));
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            0x1 | 0x8,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::{load_settings_from, save_settings_to, AppTheme, DesktopSettings, DeviceSettings};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_settings_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "audiorouter-settings-{}-{unique}.json",
            std::process::id()
        ))
    }

    #[test]
    fn settings_round_trip_preserves_user_choices() {
        let path = temporary_settings_path();
        let mut device_preferences = HashMap::new();
        device_preferences.insert(
            "lan:192.168.1.25:40400".to_string(),
            DeviceSettings {
                send_enabled: true,
                receive_enabled: false,
                send_volume: 67,
                receive_volume: 42,
            },
        );
        let settings = DesktopSettings {
            theme: AppTheme::Light,
            preconnect_controls: true,
            show_ip: false,
            lan_compressed: false,
            lan_bitrate_kbps: 256,
            bluetooth_preference: 2,
            buffer_ms: 0,
            custom_buffer_ms: 217,
            device_preferences,
            ..DesktopSettings::default()
        };

        save_settings_to(&path, &settings).expect("settings should save");
        assert_eq!(load_settings_from(&path), settings);
        fs::remove_file(path).expect("test settings file should be removed");
    }

    #[test]
    fn first_run_defaults_match_requested_choices() {
        let settings = DesktopSettings::default();
        let device = DeviceSettings::default();

        assert_eq!(settings.theme, AppTheme::System);
        assert_eq!(settings.window_width, 1180);
        assert_eq!(settings.window_height, 820);
        assert!(!settings.preconnect_controls);
        assert!(!settings.show_ip);
        assert!(!settings.lan_compressed);
        assert!(!settings.launch_on_startup);
        assert_eq!(settings.exit_behavior, "full_exit");
        assert!(!settings.restore_connection_state);
        assert!(!settings.auto_connect);
        assert_eq!(settings.auto_connect_attempts, 3);
        assert_eq!(settings.auto_connect_interval_seconds, 3);
        assert_eq!(device.send_volume, 80);
        assert_eq!(device.receive_volume, 80);
        assert!(!device.send_enabled);
        assert!(device.receive_enabled);
    }

    #[test]
    fn partial_or_out_of_range_settings_use_safe_defaults() {
        let path = temporary_settings_path();
        fs::write(
            &path,
            r#"{"theme":"light","lan_bitrate_kbps":9,"bluetooth_preference":99,"buffer_ms":500,"custom_buffer_ms":500}"#,
        )
        .expect("test settings file should be written");

        let settings = load_settings_from(&path);
        assert_eq!(settings.theme, AppTheme::Light);
        assert_eq!(settings.window_width, 1180);
        assert_eq!(settings.window_height, 820);
        assert!(!settings.preconnect_controls);
        assert_eq!(settings.lan_bitrate_kbps, 128);
        assert_eq!(settings.bluetooth_preference, 0);
        assert_eq!(settings.buffer_ms, 60);
        assert_eq!(settings.custom_buffer_ms, 240);
        fs::remove_file(path).expect("test settings file should be removed");
    }
}
