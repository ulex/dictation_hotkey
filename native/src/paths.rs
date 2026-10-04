use std::{io, path::PathBuf};
use windows::{
    core::GUID,
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_LocalAppData, FOLDERID_RoamingAppData, SHGetKnownFolderPath},
    },
};
fn folder(id: &GUID) -> io::Result<PathBuf> {
    unsafe {
        let value = SHGetKnownFolderPath(id, Default::default(), None).map_err(io::Error::other)?;
        let string = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        string.map(PathBuf::from).map_err(io::Error::other)
    }
}
pub fn config() -> io::Result<PathBuf> {
    Ok(folder(&FOLDERID_RoamingAppData)?.join("dictation_hotkey/config.json"))
}
pub fn spool() -> io::Result<PathBuf> {
    Ok(folder(&FOLDERID_LocalAppData)?.join("dictation_hotkey/spool"))
}
