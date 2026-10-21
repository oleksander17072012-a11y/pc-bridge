use std::path::PathBuf;

/// All mutable state lives in the user's local app data, never next to the binary:
/// a single-file distribution may run from a read-only or temporary directory.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("PC-Bridge");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn bin_dir() -> PathBuf {
    let dir = data_dir().join("bin");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn log_path() -> PathBuf {
    data_dir().join("pc_agent_log.txt")
}

pub fn key_path() -> PathBuf {
    data_dir().join("pairing.key")
}
