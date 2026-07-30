use std::path::{Path, PathBuf};

const TARGET_TRIPLE: &str = env!("TARGET_TRIPLE");

pub fn sidecar_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecar")
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn discover_cores() -> Vec<String> {
    discover_cores_in(&sidecar_dir())
}

fn discover_cores_in(dir: &Path) -> Vec<String> {
    let mut cores = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return cores;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_executable_file(&path) {
            continue;
        }
        let filename = entry.file_name();
        let Some(name) = filename.to_str().and_then(core_name_from_filename) else {
            continue;
        };
        if !cores.iter().any(|core| core == name) {
            cores.push(name.to_owned());
        }
    }

    cores.sort();
    cores
}

fn core_name_from_filename(filename: &str) -> Option<&str> {
    let filename = filename.strip_suffix(".exe").unwrap_or(filename);
    let target_suffix = format!("-{TARGET_TRIPLE}");
    let name = filename.strip_suffix(&target_suffix).unwrap_or(filename);
    is_safe_core_name(name).then_some(name)
}

fn is_safe_core_name(name: &str) -> bool {
    name.starts_with("verge-")
        && name.len() > "verge-".len()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub fn resolve_core_path(name: &str) -> Option<PathBuf> {
    resolve_core_path_in(&sidecar_dir(), name, cfg!(debug_assertions))
}

fn resolve_core_path_in(dir: &Path, name: &str, prefer_target_suffix: bool) -> Option<PathBuf> {
    if !is_safe_core_name(name) {
        return None;
    }

    let extension = if cfg!(windows) { ".exe" } else { "" };
    let installed = dir.join(format!("{name}{extension}"));
    let development = dir.join(format!("{name}-{TARGET_TRIPLE}{extension}"));
    let candidates = if prefer_target_suffix {
        [development, installed]
    } else {
        [installed, development]
    };

    candidates.into_iter().find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn is_valid_core(name: &str) -> bool {
    resolve_core_path(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir() -> PathBuf {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("clash-verge-discovery-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_executable(path: &Path) {
        fs::write(path, b"test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
    }

    #[test]
    fn recognizes_development_and_installed_names() {
        assert_eq!(core_name_from_filename("verge-mihomo"), Some("verge-mihomo"));
        assert_eq!(
            core_name_from_filename(&format!("verge-mihomo-tt-{TARGET_TRIPLE}")),
            Some("verge-mihomo-tt")
        );
        assert_eq!(core_name_from_filename("clash-verge-service"), None);
        assert_eq!(core_name_from_filename("verge-../mihomo"), None);
    }

    #[test]
    fn discovers_and_resolves_both_install_layouts() {
        let dir = test_dir();
        let installed = dir.join(if cfg!(windows) {
            "verge-mihomo.exe"
        } else {
            "verge-mihomo"
        });
        let development = dir.join(format!(
            "verge-mihomo-tt-{TARGET_TRIPLE}{}",
            if cfg!(windows) { ".exe" } else { "" }
        ));
        create_executable(&installed);
        create_executable(&development);

        assert_eq!(discover_cores_in(&dir), vec!["verge-mihomo", "verge-mihomo-tt"]);
        assert_eq!(resolve_core_path_in(&dir, "verge-mihomo", false), Some(installed));
        assert_eq!(resolve_core_path_in(&dir, "verge-mihomo-tt", true), Some(development));
        assert_eq!(resolve_core_path_in(&dir, "../verge-mihomo", false), None);

        fs::remove_dir_all(dir).unwrap();
    }
}
