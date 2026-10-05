use std::{io, path::PathBuf};
fn home() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("HOME is unavailable"))
}
pub fn config() -> io::Result<PathBuf> {
    Ok(home()?.join("Library/Application Support/dictation_hotkey/config.json"))
}
pub fn spool() -> io::Result<PathBuf> {
    Ok(home()?.join("Library/Caches/dictation_hotkey/spool"))
}
