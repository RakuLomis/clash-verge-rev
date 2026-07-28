use std::path::{Path, PathBuf};

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
const TARGET_TRIPLE: &str = "x86_64-unknown-linux-gnu";
#[cfg(all(target_arch = "aarch64", target_os = "linux", target_env = "gnu"))]
const TARGET_TRIPLE: &str = "aarch64-unknown-linux-gnu";
#[cfg(all(target_arch = "x86_64", target_os = "macos"))]
const TARGET_TRIPLE: &str = "x86_64-apple-darwin";
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
const TARGET_TRIPLE: &str = "aarch64-apple-darwin";
#[cfg(all(target_arch = "x86_64", target_os = "windows"))]
const TARGET_TRIPLE: &str = "x86_64-pc-windows-msvc";
#[cfg(all(target_arch = "aarch64", target_os = "windows"))]
const TARGET_TRIPLE: &str = "aarch64-pc-windows-msvc";

pub fn sidecar_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecar")
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn discover_cores() -> Vec<String> {
    let dir = sidecar_dir();
    let mut cores: Vec<String> = Vec::new();

    let suffix = format!("-{}", TARGET_TRIPLE);
    let exe_suffix = format!("-{}.exe", TARGET_TRIPLE);

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            let base = if name_str.ends_with(&exe_suffix) {
                &name_str[..name_str.len() - exe_suffix.len()]
            } else if name_str.ends_with(&suffix) {
                &name_str[..name_str.len() - suffix.len()]
            } else {
                continue;
            };

            if base.starts_with("verge-") && !cores.contains(&base.to_string()) {
                cores.push(base.to_string());
            }
        }
    }

    cores.sort();
    cores
}

pub fn resolve_core_path(name: &str) -> Option<PathBuf> {
    let dir = sidecar_dir();

    #[cfg(windows)]
    let filename = format!("{}-{}.exe", name, TARGET_TRIPLE);
    #[cfg(not(windows))]
    let filename = format!("{}-{}", name, TARGET_TRIPLE);

    let path = dir.join(&filename);
    if path.exists() { Some(path) } else { None }
}

pub fn is_valid_core(name: &str) -> bool {
    resolve_core_path(name).is_some()
}
