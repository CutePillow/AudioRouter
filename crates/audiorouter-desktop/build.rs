use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/audiorouter-icon.ico");

    #[cfg(windows)]
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    let manifest_directory = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon_path = manifest_directory.join("../../assets/icons/audiorouter-icon.ico");
    let output_directory = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let resource_script = output_directory.join("audiorouter-icon.rc");
    let resource_file = output_directory.join("audiorouter-icon.res");

    let resource_path = icon_path.to_string_lossy().replace('\\', "/");
    let script = format!("1 ICON \"{resource_path}\"\n");
    fs::write(&resource_script, script).expect("无法生成 AudioRouter 图标资源脚本");

    let resource_compiler = find_resource_compiler().expect("找不到 Windows SDK 的 rc.exe");
    let status = Command::new(resource_compiler)
        .arg("/nologo")
        .arg(format!("/fo{}", resource_file.display()))
        .arg(&resource_script)
        .status()
        .expect("无法启动 Windows 资源编译器");
    assert!(status.success(), "Windows 图标资源编译失败");

    println!(
        "cargo:rustc-link-arg-bin=audiorouter-desktop={}",
        resource_file.display()
    );
}

#[cfg(windows)]
fn find_resource_compiler() -> Option<PathBuf> {
    let candidates = [
        Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\rc.exe"),
        Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x86\rc.exe"),
    ];
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(Path::to_path_buf)
}
